//! reach-host: standalone Halo: Reach host (milestone 1 skeleton).
//! Window + D3D11 flip-model swap chain, loads MCC's haloreach.dll, runs the engine's start-up
//! sequence and supplies a host object whose every slot is logged. Output: `reach-host.log` + stdout.

mod audio;
mod browser;
mod callvote;
mod crash;
mod dormant;
mod forge;
mod host;
mod input;
mod lobby;
mod mission;
mod music;
mod log;
mod nat;
mod net;
mod options;
mod ping;
mod preview;
mod probe;
mod query;
mod playlist;
mod seamless;
mod vote;
mod profile;
mod text;
mod throttle;
mod ui;
mod upnp;

use std::ffi::c_void;
use std::mem::transmute;
use std::path::PathBuf;

use windows::core::{w, Interface, PCSTR, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW, SetDllDirectoryW};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Size of the launch options block (`s_game_options`), see notes/engine-interface.md.
const GAME_OPTIONS_SIZE: usize = options::SIZE;

struct Args {
    mcc: PathBuf,
    options: Option<PathBuf>,
    build: options::Spec,
    no_launch: bool,
    net: Option<NetArgs>,
    /// No visible window (`--hidden`, default for `--dedicated` unless `--show`).
    hidden: bool,
    /// `--size WxH` window/back-buffer size (default 1280x720; 320x180 for `--dedicated`, which nobody watches).
    size: (u32, u32),
    /// `--warp`: Direct3D 11 software rasterizer instead of the GPU (GPU-less servers).
    warp: bool,
    /// `--no-render`: set the engine's render-skip byte so the game frame never begins/presents (dedicated server).
    no_render: bool,
    /// `--probe rva,rva`: one-shot breakpoints that log their caller (RE aid, probe.rs).
    probes: Vec<usize>,
    /// `--master <url>` (hosts): register with a master server; `--server-name`, `--public-addr ip:port`.
    master: Option<String>,
    server_name: String,
    public_addr: String,
}

/// `--net-role host|client --instance N [--players N] [--bind ip:port] [--peer N=ip:port ...]
/// [--session-id hex]`, or `--local-pair host|client` (instances 0 and 1 on 127.0.0.1:47800/47801).
impl Args {
    fn dedicated(&self) -> bool {
        host::DEDICATED.load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct NetArgs {
    instance: u32,
    players: u32,
    session_id: u64,
    bind: std::net::SocketAddr,
    peers: Vec<(u64, std::net::SocketAddr)>,
}

/// Default session id shared by every instance of a game (options +0x50); any non-zero value both sides agree on.
const DEFAULT_SESSION_ID: u64 = 0x5245_4143_4853_0001;

fn local_addr(instance: u32) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], net::BASE_PORT + instance as u16))
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut mcc = std::env::var_os("MCC_PATH").map(PathBuf::from);
    let mut options = None;
    let mut no_launch = false;
    let mut build = options::Spec { mode: options::Mode::Multiplayer, difficulty: 1, skulls: 0, map: None, game_variant: None, map_variant: None, net: None };
    let mut insertion = 0i32;
    let mut role: Option<String> = None;
    let mut instance: Option<u32> = None;
    let mut players = 2u32;
    let mut session_id = DEFAULT_SESSION_ID;
    let mut bind = None;
    let mut peers: Vec<(u32, std::net::SocketAddr)> = Vec::new();
    let mut master = None;
    let mut server_name = String::from("Reach host");
    let mut public_addr = None;
    let mut join_server: Option<(String, String)> = None;
    let mut dedicated = false;
    let mut listen = false;
    let mut to_dedicated = false;
    let mut hidden = None;
    let mut probes = Vec::new();
    let mut no_render = false;
    let mut size = None;
    let mut warp = false;
    let mut forge_game = false;
    let mut it = argv.iter().cloned();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--net-role" => role = Some(it.next().ok_or("--net-role needs host|client")?),
            "--local-pair" => {
                let r = it.next().ok_or("--local-pair needs host|client")?;
                let (me, other) = if r == "host" { (0, 1) } else { (1, 0) };
                instance = Some(me);
                peers.push((other, local_addr(other)));
                role = Some(r);
            }
            // Dedicated server: instance 0, no local player, game starts at once, players join in progress.
            "--dedicated" => {
                dedicated = true;
                role = Some("host".into());
                instance = Some(0);
            }
            // Listen server (menu HOST): instance 0 with a local player, game starts at once, players join in progress.
            "--listen" => {
                listen = true;
                role = Some("host".into());
                instance = Some(0);
            }
            "--to-dedicated" => to_dedicated = true, // client of a --dedicated or --listen server
            "--max-players" => players = it.next().ok_or("--max-players needs N")?.parse().map_err(|e| format!("--max-players: {e}"))?,
            "--probe" => {
                for v in it.next().ok_or("--probe needs rva[,rva]")?.split(',') {
                    probes.push(usize::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|e| format!("--probe {v}: {e}"))?);
                }
            }
            "--no-render" => no_render = true,
            "--lobby-wait" => _ = it.next(), // read by lobby::parse
            "--playlist" => _ = it.next(), // applied by playlist::apply before parsing
            "--vote-secs" => _ = it.next(), // read by vote::parse
            "--campaign-jip" => {} // read by lobby::parse
            "--warp" => warp = true,
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                size = Some((w.parse().map_err(|e| format!("--size: {e}"))?, h.parse().map_err(|e| format!("--size: {e}"))?));
            }
            "--hidden" => hidden = Some(true),
            "--show" => hidden = Some(false),
            "--no-lazy" => dormant::DISABLED.store(true, std::sync::atomic::Ordering::Relaxed), // load the engine at once
            "--instance" => instance = Some(it.next().ok_or("--instance needs N")?.parse().map_err(|e| format!("--instance: {e}"))?),
            "--players" => players = it.next().ok_or("--players needs N")?.parse().map_err(|e| format!("--players: {e}"))?,
            "--session-id" => {
                let v = it.next().ok_or("--session-id needs a hex value")?;
                session_id = u64::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|e| format!("--session-id: {e}"))?;
            }
            "--bind" => bind = Some(it.next().ok_or("--bind needs ip:port")?.parse().map_err(|e| format!("--bind: {e}"))?),
            "--peer" => {
                let v = it.next().ok_or("--peer needs N=ip:port")?;
                let (n, a) = v.split_once('=').ok_or("--peer needs N=ip:port")?;
                peers.push((n.parse().map_err(|e| format!("--peer: {e}"))?, a.parse().map_err(|e| format!("--peer: {e}"))?));
            }
            "--mcc" => mcc = Some(PathBuf::from(it.next().ok_or("--mcc needs a value")?)),
            "--options" => options = Some(PathBuf::from(it.next().ok_or("--options needs a value")?)),
            "--map" => build.map = Some(it.next().ok_or("--map needs a value")?),
            // Forge: MCC's sandbox game variant on a map, or on a saved map variant (`--forge my_map.mvar`).
            "--forge" => {
                let v = it.next().ok_or("--forge needs a map (e.g. forge_halo) or a .mvar file")?;
                if v.to_ascii_lowercase().ends_with(".mvar") {
                    build.map = forge::map_of(std::path::Path::new(&v)).or(build.map.take());
                    build.map_variant = Some(PathBuf::from(v));
                } else {
                    build.map = Some(v);
                }
                forge_game = true;
            }
            // Firefight / campaign: notes/campaign-firefight.md.
            "--firefight" => {
                build.mode = options::Mode::Firefight;
                build.map = Some(it.next().ok_or("--firefight needs a map (e.g. ff10_prototype)")?);
            }
            "--campaign" => {
                build.mode = options::Mode::Campaign { insertion: 0 };
                build.map = Some(it.next().ok_or("--campaign needs a mission (e.g. m10)")?);
            }
            "--difficulty" => {
                let v = it.next().ok_or("--difficulty needs easy|normal|heroic|legendary")?;
                build.difficulty = match v.as_str() {
                    "easy" | "0" => 0,
                    "normal" | "1" => 1,
                    "heroic" | "2" => 2,
                    "legendary" | "3" => 3,
                    _ => return Err(format!("--difficulty {v}: expected easy|normal|heroic|legendary")),
                };
            }
            "--skulls" => build.skulls = options::parse_skulls(&it.next().ok_or("--skulls needs iron,blind,...")?)?,
            "--insertion" => insertion = it.next().ok_or("--insertion needs N")?.parse().map_err(|e| format!("--insertion: {e}"))?,
            "--game-variant" => build.game_variant = Some(PathBuf::from(it.next().ok_or("--game-variant needs a value")?)),
            "--map-variant" => build.map_variant = Some(PathBuf::from(it.next().ok_or("--map-variant needs a value")?)),
            "--master" => master = Some(it.next().ok_or("--master needs a url")?),
            "--server-name" => server_name = it.next().ok_or("--server-name needs a value")?,
            "--public-addr" => public_addr = Some(it.next().ok_or("--public-addr needs ip:port")?),
            "--browse" => _ = it.next(), // handled in main()
            "--join-server" => {
                let url = it.next().ok_or("--join-server needs <url> <index|name>")?;
                join_server = Some((url, it.next().ok_or("--join-server needs <url> <index|name>")?));
            }
            "--no-launch" => no_launch = true,
            "--test-input" | "--mouse-sens" => _ = it.next(), // read by input::init
            "--rendezvous" | "--upnp-test" => _ = it.next(), // read by nat::parse / main()
            "--no-upnp" => {}
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if let Some((url, sel)) = join_server {
        // Pick a server from the master's list and set up the usual net-role client for it.
        let mcc = mcc.as_ref().ok_or("no --mcc <dir> and MCC_PATH unset")?;
        let s = browser::pick(&browser::fetch_any(&browser::master_list_from(&url))?.1, &sel)?;
        log!("join-server: {} at {} (map {}, variant {}, {}/{} players)", s.name, s.addr, s.map, s.game_variant, s.players, s.max_players);
        let host_id = browser::parse_hex(&s.network_id)?;
        if host_id != net::net_id(0) {
            return Err(format!("server network id {host_id:#x} is not instance 0's id {:#x}; unsupported until net ids are configurable", net::net_id(0)));
        }
        let addr: std::net::SocketAddr = s.addr.parse().map_err(|e| format!("server address {}: {e}", s.addr))?;
        // A dedicated campaign refuses joins once its mission runs (lobby.rs); say so instead of a 60 s join timeout.
        if let Some(r) = &query::query_many(&[addr], std::time::Duration::from_millis(1500))[0] {
            if r.in_progress {
                return Err(lobby::IN_PROGRESS_MSG.into());
            }
            if r.lobby_secs > 0 {
                log!("join-server: campaign lobby, {} player(s) in, mission starts in {} s", r.players, r.lobby_secs);
            }
        }
        let gv = mcc.join("haloreach").join("game_variants").join(&s.game_variant);
        if !gv.is_file() {
            return Err(format!("game variant {} is not in your MCC files ({})", s.game_variant, gv.display()));
        }
        role = Some("client".into());
        // Dedicated and listen servers (menu HOST) accept any identity 1..65535 and learn our address from our
        // packets, so pick a random identity (two friends must not share one) and let the OS choose the UDP port.
        // The listing does not say which kind a player host is; the dedicated joiner layout works for both.
        to_dedicated = true;
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)
            ^ std::process::id().rotate_left(16);
        instance.get_or_insert(1 + seed % 0xFFFF);
        bind.get_or_insert(std::net::SocketAddr::from(([0, 0, 0, 0], 0)));
        peers.push((0, addr));
        session_id = browser::parse_hex(&s.session_id)?;
        players = s.max_players;
        // The listing carries no mode: ff* maps are Firefight, m<digit>* maps are campaign.
        if matches!(build.mode, options::Mode::Multiplayer) {
            let b = s.map.as_bytes();
            if s.map.starts_with("ff") {
                build.mode = options::Mode::Firefight;
            } else if b.len() > 1 && b[0] == b'm' && b[1].is_ascii_digit() {
                build.mode = options::Mode::Campaign { insertion: 0 };
            }
        }
        build.map = Some(s.map);
        build.game_variant = Some(gv);
    }
    if forge_game && build.game_variant.is_none() {
        let mcc = mcc.as_ref().ok_or("no --mcc <dir> and MCC_PATH unset")?;
        build.game_variant = Some(mcc.join("haloreach").join("game_variants").join(forge::FORGE_GAME_VARIANT));
    }
    // A saved variant from our library: its sidecar names the map (shown in the browser and the hosting banner).
    if build.map.is_none() {
        build.map = build.map_variant.as_deref().and_then(forge::map_of);
    }
    if let options::Mode::Campaign { insertion: i } = &mut build.mode {
        *i = insertion;
    }
    if options.is_some() && build.wanted() {
        return Err("--options and --map/--game-variant/--map-variant are exclusive".into());
    }
    let net = match role.as_deref() {
        None => None,
        Some(r @ ("host" | "client")) => {
            let instance = instance.unwrap_or(if r == "host" { 0 } else { 1 });
            if (r == "host") != (instance == 0) {
                return Err("instance 0 is the host; clients use instance 1..".into());
            }
            // On a dedicated server a client's instance number is only its identity (net id / xuid), any 1..65535.
            if players < 2 || players > 16 || (instance >= players && !to_dedicated) || instance > 0xFFFF {
                return Err(format!("--players {players} must be 2..16 and above --instance {instance}"));
            }
            if build.map.is_none() && build.map_variant.is_none() {
                return Err("a networked game needs --map or --map-variant (not --options)".into());
            }
            build.net = Some(options::NetGame { instance, players, session_id, dedicated: dedicated || to_dedicated, listen });
            let peers = peers.into_iter().map(|(n, a)| (net::net_id(n), a)).collect();
            Some(NetArgs { instance, players, session_id, bind: bind.unwrap_or(local_addr(instance)), peers })
        }
        Some(r) => return Err(format!("--net-role {r}: expected host or client")),
    };
    if to_dedicated && build.net.is_none() {
        return Err("--to-dedicated needs --net-role client".into());
    }
    throttle::parse(argv)?;
    nat::parse(argv)?;
    lobby::parse(argv)?;
    vote::parse(argv)?;
    if dedicated && matches!(build.mode, options::Mode::Campaign { .. }) {
        lobby::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        lobby::MAX_PLAYERS.store(players, std::sync::atomic::Ordering::Relaxed);
    }
    // Listen host (menu HOST CO-OP) for campaign: gather friends before the mission (lobby.rs), up to 120 s by default.
    if listen && matches!(build.mode, options::Mode::Campaign { .. }) {
        lobby::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        lobby::LISTEN.store(true, std::sync::atomic::Ordering::Relaxed);
        lobby::MAX_PLAYERS.store(players, std::sync::atomic::Ordering::Relaxed);
        if !argv.iter().any(|a| a == "--lobby-wait") {
            lobby::WAIT_SECS.store(120, std::sync::atomic::Ordering::Relaxed);
        }
    }
    host::DEDICATED.store(dedicated, std::sync::atomic::Ordering::Relaxed);
    host::LISTEN.store(listen, std::sync::atomic::Ordering::Relaxed);
    Ok(Args {
        hidden: hidden.unwrap_or(dedicated),
        probes,
        no_render,
        warp,
        size: size.unwrap_or(if dedicated { (320, 180) } else { (1280, 720) }),
        mcc: mcc.ok_or("no --mcc <dir> and MCC_PATH unset")?,
        options,
        build,
        no_launch,
        net,
        // `--master ""` (the menu passes its list, which may be empty) or a list that resolves to nothing: no master.
        master: master.filter(|m| !browser::master_list_from(m).is_empty()),
        server_name,
        public_addr: public_addr.unwrap_or_default(),
    })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(r) = ui::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    if let Some(r) = input::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe fn create_window(hidden: bool, (w, h): (u32, u32)) -> windows::core::Result<HWND> {
    let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
    let class = w!("ReachHostWindow");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        ..Default::default()
    };
    RegisterClassW(&wc);
    let mut r = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
    let style = if hidden { WS_OVERLAPPEDWINDOW } else { WS_OVERLAPPEDWINDOW | WS_VISIBLE };
    AdjustWindowRect(&mut r, style, false)?;
    CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        w!("Noble Outreach"),
        style,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        r.right - r.left,
        r.bottom - r.top,
        None,
        None,
        Some(hinst.into()),
        None,
    )
}

struct Gfx {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swapchain: IDXGISwapChain1,
}

unsafe fn create_gfx(hwnd: HWND, (w, h): (u32, u32), warp: bool) -> windows::core::Result<Gfx> {
    let mut device = None;
    let mut context = None;
    D3D11CreateDevice(
        None,
        if warp { D3D_DRIVER_TYPE_WARP } else { D3D_DRIVER_TYPE_HARDWARE },
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )?;
    let device = device.unwrap();
    let context = context.unwrap();
    let dxgi: IDXGIDevice = device.cast()?;
    let adapter = dxgi.GetAdapter()?;
    let factory: IDXGIFactory2 = adapter.GetParent()?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: w,
        Height: h,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        // The engine also samples the back buffer (its own swap chain uses RENDER_TARGET_OUTPUT | SHADER_INPUT).
        BufferUsage: DXGI_USAGE(DXGI_USAGE_RENDER_TARGET_OUTPUT.0 | DXGI_USAGE_SHADER_INPUT.0),
        BufferCount: 2,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        ..Default::default()
    };
    let swapchain = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
    Ok(Gfx { device, context, swapchain })
}

type CreateFn = unsafe extern "system" fn(*mut *mut c_void) -> u64;
type SetSettingsFn = unsafe extern "system" fn(*const u16);
type InitGfxFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> usize;
type InitGameFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8) -> HANDLE;
type PostMessageFn = unsafe extern "system" fn(*mut c_void, u32, *const u8) -> *mut c_void;

/// Engine message 14: the engine re-reads the window size and (re)builds its render targets.
const ENGINE_MSG_RESIZE: u32 = 14;

fn run() -> Result<(), String> {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    vote::await_previous(); // relaunched after a vote restart: the old process must free its UDP port first
    playlist::apply(&mut argv)?; // --playlist: this process plays one entry (mode flags rewritten)
    preview::apply_args(&mut argv)?; // --preview: hidden armour-preview engine for the menu's CUSTOMIZE screen
    let mut args = parse_args(&argv)?;
    log!("mcc dir: {}", args.mcc.display());
    // No launch arguments: show the main menu first (ui/menu.rs); it hands back the arguments of the chosen game.
    let mut menu = args.options.is_none() && !args.build.wanted() && args.net.is_none() && !args.no_launch && !args.hidden && !args.dedicated();
    // REACH_FROM_MENU: a game started from the menu, also after mission.rs relaunched it into the next mission (it
    // returns to the menu at the end like the first one).
    let mut from_menu = std::env::var_os("REACH_FROM_MENU").is_some();
    let player = ui::settings::get();
    if !argv.iter().any(|a| a == "--mouse-sens") {
        input::set_mouse_sens(player.mouse_sens as f64);
    }
    if menu {
        ui::set_active(true); // before input::init, so it does not grab the mouse
        if let Some(m) = &args.master {
            std::env::set_var("REACH_MASTER", m); // browser::master_list; inherited by a relaunched menu
        }
    }
    // What the menu starts from when a game returns to it in this process (as `relaunch_to_menu` would start it).
    let menu_argv = vec!["--mcc".to_string(), args.mcc.to_string_lossy().into_owned()];
    unsafe {
        crash::install();
        log!("vectored exception handler installed");

        // Lazy engine start (dormant.rs): an empty dedicated server waits with only its socket, query and master
        // heartbeat; the first join (or wake packet) loads the engine below.
        if args.dedicated() && args.net.is_some() && !args.no_launch && !dormant::DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
            start_networking(&args)?;
            dormant::wait();
        }

        // REACH_UI_HIDDEN: unattended checks keep a relaunched menu hidden too.
        let hwnd = create_window(args.hidden || std::env::var_os("REACH_UI_HIDDEN").is_some(), args.size).map_err(|e| format!("create window: {e}"))?;
        log!("window created {hwnd:?}{} in process {}", if args.hidden { " (hidden)" } else { "" }, std::process::id());
        input::init(hwnd);
        // Under Wine/DXVK the swap chain sometimes fails at start (E_FAIL) and works on the next try, more often when
        // two of our processes start at once (menu + preview engine).
        let mut tries = 0;
        let gfx = loop {
            match create_gfx(hwnd, args.size, args.warp) {
                Ok(g) => break g,
                Err(e) if tries < 4 => {
                    tries += 1;
                    log!("d3d11/swap chain: {e}; retry {tries}");
                    std::thread::sleep(std::time::Duration::from_millis(300 * tries));
                }
                Err(e) => return Err(format!("d3d11/swap chain: {e}")),
            }
        };
        host::WINDOW.store(hwnd.0 as usize, std::sync::atomic::Ordering::Relaxed);
        host::SWAPCHAIN.store(gfx.swapchain.as_raw() as usize, std::sync::atomic::Ordering::Relaxed);
        log!(
            "D3D11 device {:?}, context {:?}, flip-model swap chain {:?}",
            gfx.device.as_raw(),
            gfx.context.as_raw(),
            gfx.swapchain.as_raw()
        );

        // The engine resolves `haloreach\maps\...` relative to the MCC folder.
        let dll_dir = args.mcc.join("haloreach");
        SetDllDirectoryW(PCWSTR(wide(&dll_dir.to_string_lossy()).as_ptr())).map_err(|e| format!("SetDllDirectory: {e}"))?;
        std::env::set_current_dir(&args.mcc).map_err(|e| format!("set cwd: {e}"))?;
        log!("SetDllDirectory({}), cwd = {}", dll_dir.display(), args.mcc.display());

        // The engine delay-loads Bink (bink2w64.dll, in MCC's own binaries folder) and first touches it when a
        // level is unloaded (end of a game). Not found there = fatal delay-load exception 0xc06d007e.
        let bink = args.mcc.join("MCC").join("Binaries").join("Win64").join("bink2w64.dll");
        match LoadLibraryW(PCWSTR(wide(&bink.to_string_lossy()).as_ptr())) {
            Ok(_) => log!("preloaded {}", bink.display()),
            Err(e) => log!("could not preload {}: {e} (level unloads will fail)", bink.display()),
        }
        if !preview::is_child() {
            audio::preload(&args.mcc); // the preview child stays silent (no XAudio2 = no game audio)
        }

        // Game after game in this window (seamless.rs): each pass plays one game (after the menu, if shown).
        loop {
            if menu {
                let _ = preview::MCC.set(args.mcc.clone());
                music::start(&args.mcc); // menu theme from the player's install; fades out below
                let choice = ui::menu::run(hwnd, &gfx.swapchain, &gfx.context, &args.mcc);
                music::stop();
                preview::stop(); // the menu's preview engine is not needed in game
                match choice {
                    ui::menu::Choice::Quit => {
                        log!("menu: quit");
                        return Ok(());
                    }
                    ui::menu::Choice::Launch(extra) => {
                        argv.extend(extra);
                        args = parse_args(&argv)?;
                        let _ = vote::LAUNCH_ARGS.set(argv.clone()); // a vote restart relaunches with the menu's flags
                        from_menu = true;
                        std::env::set_var("REACH_FROM_MENU", "1");
                        if std::env::var_os("REACH_NAME").is_none() {
                            std::env::set_var("REACH_NAME", &ui::settings::get().player_name); // read by slot 88
                        }
                    }
                }
            }
            seamless::set_argv(&argv);
            let n = seamless::GAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            log!("seamless: game {n} in process {} window {hwnd:?}: {}; {}", std::process::id(), argv.join(" "), seamless::mem_line());
            let next = play_game(&args, &argv, from_menu, &gfx, &dll_dir)?;
            log!("seamless: game {n} over, next {next:?}; {}", seamless::mem_line());
            reset_game_state();
            match next {
                seamless::Next::Menu(notice) => {
                    stop_networking(); // the menu hosts nothing; the next game binds afresh
                    match &notice {
                        Some(n) => std::env::set_var("REACH_UI_NOTICE", n),
                        None => std::env::remove_var("REACH_UI_NOTICE"),
                    }
                    // Unattended checks: REACH_SEAMLESS_GAMES=n replays the REACH_UI_SHOT steps for n games, then
                    // the menu takes its "back from a game" capture and quits.
                    let want = std::env::var("REACH_SEAMLESS_GAMES").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
                    if n >= want {
                        std::env::set_var("REACH_UI_RELAUNCHED", "1");
                    }
                    std::env::remove_var("REACH_FROM_MENU");
                    argv = menu_argv.clone();
                    args = parse_args(&argv)?;
                    from_menu = false;
                    menu = true;
                }
                seamless::Next::Game(a) => {
                    argv = a;
                    args = parse_args(&argv)?;
                    menu = false;
                }
            }
        }
    }
}

/// Per-game state of our modules, cleared between two games in one process.
fn reset_game_state() {
    ui::overlay::reset();
    input::reset_game();
    ui::scoreboard::reset();
    mission::reset();
    lobby::reset();
    net::reset_game();
    vote::end_follow();
    callvote::reset();
    *host::NEXT_GAME_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    host::DEDICATED.store(false, std::sync::atomic::Ordering::Relaxed);
    host::LISTEN.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Load the engine, play one game in our window, shut the engine down and unload it. Returns what comes next.
/// Paths that cannot (yet) continue in this process relaunch it as before and do not return.
unsafe fn play_game(args: &Args, argv: &[String], from_menu: bool, gfx: &Gfx, dll_dir: &std::path::Path) -> Result<seamless::Next, String> {
    // Solo or hosted campaign from the menu: roll on into the next mission when this one ends (mission.rs).
    if from_menu && !args.dedicated() && matches!(args.build.mode, options::Mode::Campaign { .. }) && args.net.as_ref().is_none_or(|n| n.instance == 0) {
        if let Some(map) = &args.build.map {
            let difficulty = ["easy", "normal", "heroic", "legendary"][args.build.difficulty.clamp(0, 3) as usize];
            mission::set_current((map.clone(), difficulty.into(), args.mcc.clone()));
        }
    }

    // Client: wake a dormant dedicated server now, so its engine loads while ours does (dormant.rs).
    // REACH_NO_WAKE=1: behave like an older client (no wake, no wait), to check the engine's own join retry.
    let join_host = args.net.as_ref().filter(|n| n.instance != 0 && std::env::var_os("REACH_NO_WAKE").is_none()).and_then(|n| n.peers.iter().find(|p| p.0 == net::net_id(0)).map(|p| p.1));
    if let Some(h) = join_host {
        dormant::send_wake(h);
    }
    let dll_path = dll_dir.join("haloreach.dll");
    let module = LoadLibraryW(PCWSTR(wide(&dll_path.to_string_lossy()).as_ptr()))
        .map_err(|e| format!("LoadLibrary {}: {e}", dll_path.display()))?;
    let base = module.0 as usize;
    crash::ENGINE_BASE.store(base, std::sync::atomic::Ordering::Relaxed);
    net::ENGINE_BASE.store(base, std::sync::atomic::Ordering::Relaxed);
    probe::arm(base, &args.probes);
    audio::watch(base);
    input::install_cursor_hooks(base); // engine menus (loadout picker) read the OS cursor
    if args.no_render {
        // The render thread's frame function (0x1800c2dfc) only begins, draws and presents a frame while this
        // byte is 0; nothing in the engine writes it. The loading screen still draws through its own path.
        // Costs more CPU, not less: the engine's render thread then spins at 100 % of a core (nothing paces it).
        *((base + 0x263ea75) as *mut u8) = 1;
        log!("--no-render: engine render-skip byte haloreach+0x263ea75 set");
    }
    if args.dedicated() {
        throttle::install(base); // game thread has no frame limiter (notes/dedicated.md)
        lobby::install_jip(base);
    } else if args.net.as_ref().is_some_and(|n| n.instance == 0) {
        throttle::install_tick_only(base); // game-thread tick that drops peers saying bye (net.rs)
    }
    log!("loaded {} at {base:#x}", dll_path.display());
    let get = |name: &'static [u8]| -> Result<usize, String> {
        GetProcAddress(module, PCSTR(name.as_ptr()))
            .map(|f| f as usize)
            .ok_or_else(|| format!("missing export {}", String::from_utf8_lossy(&name[..name.len() - 1])))
    };
    let set_settings: SetSettingsFn = transmute(get(b"SetLibrarySettings\0")?);
    let create_data: CreateFn = transmute(get(b"CreateDataAccess\0")?);
    let create_engine: CreateFn = transmute(get(b"CreateGameEngine\0")?);
    log!("exports resolved (SLS +{:#x}, CDA +{:#x}, CGE +{:#x})",
        set_settings as usize - base, create_data as usize - base, create_engine as usize - base);

    let mut settings = [0u16; 3 * 85];
    for i in 0..3 {
        for (j, c) in "en-US".encode_utf16().enumerate() {
            settings[i * 85 + j] = c;
        }
    }
    set_settings(settings.as_ptr());
    log!("SetLibrarySettings(en-US x3) returned");

    let mut data: *mut c_void = std::ptr::null_mut();
    let rc = create_data(&mut data);
    log!("CreateDataAccess -> {rc:#x}, object {data:?}");
    let mut engine: *mut c_void = std::ptr::null_mut();
    let rc = create_engine(&mut engine);
    log!("CreateGameEngine -> {rc:#x}, object {engine:?}");
    if engine.is_null() {
        return Err("no engine object".into());
    }
    let vt = *(engine as *const *const usize);
    log!("engine vtable {vt:?}, slots 1 = haloreach+{:#x}, 2 = haloreach+{:#x}",
        *vt.add(1) - base, *vt.add(2) - base);

    preview::ENGINE.store(engine as usize, std::sync::atomic::Ordering::Relaxed);
    preview::COMMAND.store(*vt.add(9), std::sync::atomic::Ordering::Relaxed);
    let init_gfx: InitGfxFn = transmute(*vt.add(1));
    let r = init_gfx(engine, gfx.device.as_raw(), gfx.context.as_raw(), gfx.swapchain.as_raw(), std::ptr::null_mut());
    log!("initialize_graphics returned {r:#x}");

    if args.no_launch {
        log!("--no-launch: stopping after engine creation");
        std::process::exit(0);
    }

    let options: Vec<u8> = match &args.options {
        None if args.build.wanted() => {
            let b = options::build(&args.build, &args.mcc, data, base)?;
            let out = exe_dir().join("built-options.bin");
            match std::fs::write(&out, &b) {
                Ok(()) => log!("built options written to {}", out.display()),
                Err(e) => log!("could not write {}: {e}", out.display()),
            }
            b
        }
        Some(p) => {
            let b = std::fs::read(p).map_err(|e| format!("read {}: {e}", p.display()))?;
            if b.len() != GAME_OPTIONS_SIZE {
                return Err(format!("{} is {:#x} bytes, expected {GAME_OPTIONS_SIZE:#x}", p.display(), b.len()));
            }
            log!("launch options from {}", p.display());
            b
        }
        None => {
            // PLACEHOLDER: not a real launch. Zeroed block, game_mode = 3 (+0x0C), flags bit 3 (multiplayer).
            let mut b = vec![0u8; GAME_OPTIONS_SIZE];
            b[0] |= 0x08;
            b[0x0C..0x10].copy_from_slice(&3u32.to_le_bytes());
            log!("PLACEHOLDER launch options (zeroed, game_mode=3, flags bit 3): expect the engine to fail");
            b
        }
    };

    start_networking(args)?; // already up on a lazy dedicated server (dormant.rs), or kept from the last game
    if args.dedicated() || host::LISTEN.load(std::sync::atomic::Ordering::Relaxed) {
        *host::NEXT_GAME_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()) = Some(options.clone());
    }
    if let Some(h) = join_host {
        dormant::wait_for_host(h, std::time::Duration::from_secs(90)); // a woken server needs a few s to reach state 8
    }
    let host = host::build_host();
    log!("host object {host:?}, vtable {:?}", (*host).vtable);
    let init_game: InitGameFn = transmute(*vt.add(2));
    let thread = init_game(engine, host as *mut c_void, options.as_ptr());
    log!("initialize_game returned thread handle {thread:?}");

    // Render targets are only built after a resize message; MCC sends one once the game runs.
    let post: PostMessageFn = transmute(*vt.add(3));
    post(engine, ENGINE_MSG_RESIZE, std::ptr::null());
    ui::overlay::ENGINE.store(engine as usize, std::sync::atomic::Ordering::Relaxed);
    ui::overlay::POST.store(*vt.add(3), std::sync::atomic::Ordering::Relaxed);
    log!("posted engine message {ENGINE_MSG_RESIZE} (resize)");

    let seamless = seamless::enabled() && !args.dedicated() && !preview::is_child();
    let mut exited = thread.is_invalid();
    if std::env::var_os("REACH_SAMPLE").is_some() && !exited {
        sample_game_thread(thread.0 as usize, base); // debug aid: where does the game thread spend its time
    }
    let mut cpu = CpuLog::new();
    let mut msg = MSG::default();
    let join_t0 = std::time::Instant::now();
    let next = loop {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                log!("window closed");
                net::send_bye(); // peers drop us now, not after the engine's ~16 s timeout
                ui::hard_exit(0); // a wedged engine thread must not keep the process alive
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !exited && WaitForSingleObject(thread, 0) == WAIT_OBJECT_0 {
            let mut code = 0u32;
            let _ = windows::Win32::System::Threading::GetExitCodeThread(thread, &mut code);
            log!("game thread exited, code {code:#x}");
            exited = true;
            if preview::is_child() {
                std::process::exit(4); // preview scene over (Slayer time limit): the menu starts a fresh one
            }
            if args.dedicated() {
                // Once the last player has left, the engine cannot start another game with nobody in it: its
                // life cycle falls back to the main menu, which a hosted engine turns into an exit. The server
                // process exits and its supervisor (run script loop, docker restart policy) starts a fresh one that
                // waits for the next first player (a server has no window to keep).
                log!("dedicated: engine stopped; exiting with code 3 so the supervisor restarts the server");
                std::process::exit(3);
            }
            if let Some(next) = mission::relaunch_args(argv) {
                // Mission over: the next one in this window (or a fresh process with REACH_RELAUNCH=1, mission.rs).
                if seamless {
                    log!("mission: next mission in this process with {}", next.join(" "));
                    break seamless::Next::Game(next);
                }
                let exe = std::env::current_exe().unwrap_or_default();
                let _ = std::fs::copy(exe.with_file_name("reach-host.log"), exe.with_file_name("reach-host-game.log"));
                match std::process::Command::new(&exe).args(&next).env("REACH_VOTE_PREV_PID", std::process::id().to_string()).spawn() {
                    Ok(c) => {
                        log!("mission: next mission in process {} with {}", c.id(), next.join(" "));
                        ui::hard_exit(0);
                    }
                    Err(e) => log!("mission: could not start the next mission: {e}"),
                }
            }
            if let Some(n) = seamless::take_next() {
                break n; // a vote follow decided while the thread was ending
            }
            if (from_menu || ui::overlay::leave_requested().is_some()) && !vote::following() {
                if seamless {
                    break seamless::Next::Menu(None);
                }
                ui::relaunch_to_menu(&args.mcc);
            }
        }
        // A vote follow (vote.rs) or a mission chain decided the next game while this one still runs.
        if seamless && seamless::next_pending() {
            break seamless::take_next().unwrap();
        }
        // LEAVE GAME: say bye (the host drops us at once; a listen host's joiners go back to their menus), then
        // return to the menu: in this window (the engine is shut down below), or in a fresh process (REACH_RELAUNCH=1).
        if ui::overlay::leave_requested().is_some() {
            net::send_bye();
            log!("leave game: returning to the menu");
            if seamless {
                break seamless::Next::Menu(None);
            }
            ui::relaunch_to_menu(&args.mcc);
        }
        // Joiner that never heard from the host (unreachable address, carrier-grade NAT, firewall): back to the
        // menu with a reason instead of a black screen forever.
        if args.net.as_ref().is_some_and(|n| n.instance != 0) && !net::host_heard() && join_t0.elapsed().as_secs() >= 20 && !vote::following() {
            log!("join: no answer from the host in 20 s; returning to the menu");
            net::send_bye();
            let why = "Could not reach the host. If they host from home behind carrier-grade NAT (CGNAT), nobody can reach them: let another player host, or play on a server.";
            if seamless {
                break seamless::Next::Menu(Some(why.into()));
            }
            ui::relaunch_to_menu_with(&args.mcc, Some(why));
        }
        if net::HOST_LEFT.load(std::sync::atomic::Ordering::Relaxed) && !vote::following() {
            log!("host left the game: returning to the menu");
            // Kicked by a call vote: say so instead.
            let why = callvote::kicked().unwrap_or_else(|| "The host left the game.".into());
            if seamless {
                break seamless::Next::Menu(Some(why));
            }
            ui::relaunch_to_menu_with(&args.mcc, Some(&why));
        }
        cpu.tick();
        if lobby::LISTEN.load(std::sync::atomic::Ordering::Relaxed) {
            hold_launch_for_first_player(base, args.net.as_ref().map_or(0, |n| n.players)); // state-8 timers
        }
        if args.dedicated() {
            hold_launch_for_first_player(base, args.net.as_ref().map_or(0, |n| n.players));
            let launch = ((base + 0xc1a114) as *const u32).read_volatile();
            dormant::tick(launch);
            // Woken (by a stray packet, or a client that gave up) but nobody came: back to dormant via the supervisor.
            if launch == 8 && dormant::since_wake().is_some_and(|t| t.as_secs() >= 120) && net::active_peer_addrs(std::time::Duration::from_secs(15)).is_empty() {
                log!("dormant: nobody joined within 120 s of the wake; exiting with code 3 so the supervisor restarts the server dormant");
                std::process::exit(3);
            }
            vote::tick(); // playlist vote (no-op without --playlist)
            keep_session_without_local_users(base);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    // End the engine in place: ask the game loop to stop, wait for the game thread, destroy the engine objects and
    // unload the DLL. If the thread does not end, fall back to a fresh process (the old way).
    if !exited {
        seamless::request_exit(base, b"host: game over\0");
        if !seamless::wait_thread(thread, std::time::Duration::from_secs(20)) {
            return Ok(relaunch_fallback(args, next));
        }
    }
    match &next {
        seamless::Next::Game(a) => ui::loading::show(&gfx.swapchain, &gfx.context, "Loading", ui::loading::map_of(a).as_deref(), &args.mcc),
        seamless::Next::Menu(_) => ui::loading::show(&gfx.swapchain, &gfx.context, "Leaving the game", None, &args.mcc),
    }
    let _ = windows::Win32::Foundation::CloseHandle(thread);
    ui::overlay::ENGINE.store(0, std::sync::atomic::Ordering::Relaxed);
    ui::overlay::POST.store(0, std::sync::atomic::Ordering::Relaxed);
    preview::ENGINE.store(0, std::sync::atomic::Ordering::Relaxed);
    let destroy: unsafe extern "system" fn(*mut c_void) = transmute(*vt);
    destroy(engine);
    // The data-access object is left alone: its destructor first frees a cached helper whose teardown asks whether
    // the game's scenario is still valid, reading game-thread state that is gone (access violation at
    // haloreach+0x586e8, caught inside the engine). A few hundred KB per game; the DLL unload drops the rest.
    let _ = data;
    let size = seamless::image_size(base);
    let left = seamless::engine_threads(base, size);
    if !left.is_empty() {
        log!("seamless: {} engine thread(s) still running ({}); the DLL stays loaded",
            left.len(), left.iter().map(|(t, s)| format!("tid {t} start +{s:#x}")).collect::<Vec<_>>().join(", "));
        return Ok(relaunch_fallback(args, next));
    }
    crash::ENGINE_BASE.store(0, std::sync::atomic::Ordering::Relaxed);
    net::ENGINE_BASE.store(0, std::sync::atomic::Ordering::Relaxed);
    if !seamless::unload(module, &dll_path) {
        return Ok(relaunch_fallback(args, next));
    }
    // Leak check: references the engine still holds on our device (its textures and buffers keep one each).
    let refs = {
        let raw = Interface::as_raw(&gfx.device);
        let vt = Interface::vtable(&gfx.device);
        let n = (vt.base__.AddRef)(raw);
        (vt.base__.Release)(raw);
        n - 1
    };
    log!("seamless: engine unloaded; D3D device references {refs}; {}", seamless::mem_line());
    if seamless::over_memory_limit() {
        log!("seamless: working set over the limit (engine leaks): next game in a fresh process");
        return Ok(relaunch_fallback(args, next));
    }
    Ok(next)
}

/// The engine could not be ended in place: carry on in a fresh process, as before seamless games.
fn relaunch_fallback(args: &Args, next: seamless::Next) -> seamless::Next {
    log!("seamless: falling back to a fresh process");
    match next {
        seamless::Next::Menu(notice) => ui::relaunch_to_menu_with(&args.mcc, notice.as_deref()),
        seamless::Next::Game(a) => {
            let exe = std::env::current_exe().unwrap_or_default();
            let _ = std::fs::copy(exe.with_file_name("reach-host.log"), exe.with_file_name("reach-host-game.log"));
            match std::process::Command::new(&exe).args(&a).env("REACH_VOTE_PREV_PID", std::process::id().to_string()).spawn() {
                Ok(c) => log!("seamless: next game in process {} with {}", c.id(), a.join(" ")),
                Err(e) => log!("seamless: could not start the next game: {e}"),
            }
            ui::hard_exit(0);
        }
    }
}

/// Our UDP transport, the info query, the playlist vote server and the master heartbeat: everything a host needs
/// before (or, on a lazy dedicated server, without) the engine. A transport already running with the same identity
/// and address is kept (lazy dedicated server; a vote follow or the next mission in this process, seamless.rs);
/// any other is closed first.
fn start_networking(args: &Args) -> Result<(), String> {
    let same = args.net.as_ref().is_some_and(|n| {
        let host = n.peers.iter().find(|p| p.0 == net::net_id(0)).map(|p| p.1).filter(|_| n.instance != 0);
        net::running_as(net::net_id(n.instance), n.bind, host)
    });
    if same {
        log!("network game: keeping the running transport");
        if let Some(n) = args.net.as_ref().filter(|n| n.instance == 0) {
            set_query_info(args, n); // the next mission's map
        }
        return Ok(());
    }
    stop_networking();
    if let Some(n) = &args.net {
        log!("network game: instance {} of {} ({}), session id {:#x}, xuid {:#x}",
            n.instance, n.players, if n.instance == 0 { "host" } else { "client" }, n.session_id, options::xuid(n.instance));
        host::LOCAL_XUID.store(options::xuid(n.instance), std::sync::atomic::Ordering::Relaxed);
        net::start(net::net_id(n.instance), n.bind, n.peers.clone())?;
        ping::start(n.instance, n.players); // Tab scoreboard ping (host pings, clients answer)
        if n.instance == 0 {
            set_query_info(args, n);
            vote::start_server(); // no-op without --playlist
        }
        if let (Some(master), 0) = (&args.master, n.instance) {
            let name_of = |p: &Option<PathBuf>| p.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
            let map = args.build.map.clone().or_else(|| name_of(&args.build.map_variant)).unwrap_or_default();
            let addr = if args.public_addr.is_empty() { format!(":{}", n.bind.port()) } else { args.public_addr.clone() };
            browser::start_heartbeat(browser::Registration {
                master: master.clone(),
                info: browser::ServerInfo {
                    name: args.server_name.clone(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    addr,
                    network_id: format!("{:x}", net::net_id(0)),
                    session_id: format!("{:x}", n.session_id),
                    map,
                    game_variant: name_of(&args.build.game_variant).unwrap_or_else(|| if args.build.mode == options::Mode::Firefight { "hr_ff_standard_1set.bin" } else { "slayer_054.bin" }.into()),
                    players: 1,
                    max_players: n.players,
                    dedicated: host::DEDICATED.load(std::sync::atomic::Ordering::Relaxed),
                },
            });
            log!("master: heartbeat to {master} started");
        }
        if host::LISTEN.load(std::sync::atomic::Ordering::Relaxed) {
            // NAT punch registration + UPnP (nat.rs) unless a tunnel/forwarded --public-addr was given.
            if args.public_addr.is_empty() {
                let masters = args.master.as_deref().map_or_else(browser::master_list, browser::master_list_from);
                nat::start_host(n.bind.port(), masters);
            }
            ui::hostinfo::start(n.bind.port(), args.master.clone(), args.public_addr.clone());
        }
        if n.instance != 0 {
            // Joiner: check the host answers directly, else ask the rendezvous for a NAT punch (nat.rs).
            if let Some((_, addr)) = n.peers.iter().find(|(id, _)| *id == net::net_id(0)) {
                nat::start_client(*addr, browser::master_list());
            }
        }
    }
    Ok(())
}

/// What the UDP info query answers for a hosting instance (query.rs).
fn set_query_info(args: &Args, n: &NetArgs) {
    // Any hosting instance answers the browser's UDP info query (query.rs), master or not.
    let file = |p: &Option<PathBuf>| p.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
    let mode = match args.build.mode {
        options::Mode::Firefight => query::MODE_FIREFIGHT,
        options::Mode::Campaign { .. } => query::MODE_CAMPAIGN,
        options::Mode::Multiplayer if args.build.game_variant.as_deref().is_some_and(forge::is_forge_file) => query::MODE_FORGE,
        options::Mode::Multiplayer => query::MODE_MULTIPLAYER,
    };
    query::set_info(query::Info {
        name: args.server_name.clone(),
        version: env!("CARGO_PKG_VERSION").into(),
        map: args.build.map.clone().or_else(|| file(&args.build.map_variant)).unwrap_or_default(),
        // Same fallback the master heartbeat registers when no --game-variant was given.
        game_variant: file(&args.build.game_variant).unwrap_or_else(|| if mode == query::MODE_FIREFIGHT { "hr_ff_standard_1set.bin" } else { "slayer_054.bin" }.into()),
        mode,
        max_players: n.players.min(255) as u8,
        dedicated: host::DEDICATED.load(std::sync::atomic::Ordering::Relaxed),
        network_id: net::net_id(0),
        session_id: n.session_id,
        playlist: playlist::get().map(|p| p.name.clone()).unwrap_or_default(),
    });
}

/// Close the transport and its side channels (seamless.rs: back to the menu, or a game with other network flags).
fn stop_networking() {
    if net::running() {
        net::stop();
        nat::stop();
        ui::hostinfo::stop();
    }
}

/// Debug aid (`REACH_SAMPLE=1`): suspend the game thread every 5 ms, record where it is (engine RVA of rip plus
/// the first engine-looking stack words) and log a histogram every 10 s.
fn sample_game_thread(thread: usize, base: usize) {
    use windows::Win32::System::Diagnostics::Debug::{GetThreadContext, CONTEXT, CONTEXT_FLAGS};
    use windows::Win32::System::Threading::{ResumeThread, SuspendThread};
    std::thread::spawn(move || unsafe {
        let h = HANDLE(thread as *mut c_void);
        let mut hist: std::collections::HashMap<String, u32> = Default::default();
        let mut last = std::time::Instant::now();
        let mut ctx: Box<CONTEXT> = Box::new(std::mem::zeroed());
        loop {
            std::thread::sleep(std::time::Duration::from_millis(5));
            ctx.ContextFlags = CONTEXT_FLAGS(0x10001F); // CONTEXT_ALL (amd64)
            if SuspendThread(h) == u32::MAX {
                continue;
            }
            let ok = GetThreadContext(h, &mut *ctx).is_ok();
            let (rip, rsp) = (ctx.Rip as usize, ctx.Rsp as usize);
            let mut key = String::new();
            if ok {
                let rva = |a: usize| if a >= base && a < base + 0x6000000 { format!("{:#x}", a - base) } else { format!("ext:{:#x}", a & 0xfffff) };
                key += &rva(rip);
                let mut n = 0;
                for i in 0..96 {
                    let w = *((rsp + i * 8) as *const usize);
                    if w >= base && w < base + 0x6000000 && n < 4 {
                        key += &format!(" <{}", rva(w));
                        n += 1;
                    }
                }
            }
            ResumeThread(h);
            *hist.entry(key).or_default() += 1;
            if last.elapsed().as_secs() >= 10 {
                last = std::time::Instant::now();
                let mut v: Vec<_> = hist.drain().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                for (k, c) in v.iter().take(8) {
                    log!("sample {c}: {k}");
                }
            }
        }
    });
}

/// Dedicated server: launch state 8 waits until the session's joined count (`GetSessionJoinedPlayerCount`, which
/// counts machines, so the server alone is 1) reaches options +0xE8 = 2, i.e. the first remote player, but gives
/// up after 45 s (60 s per state, 90 s overall). Keep those launch timers fresh while the server waits.
/// On entering the wait it also sets the session's max players (`SetSessionMaxPlayers` 0x18038e888, ecx = count,
/// fails if more are already in): Firefight servers leave options +0x01 bit 6 clear, so +0x03 is never applied
/// and the session would otherwise admit 16 (5 joined a "max 4" Firefight server).
unsafe fn hold_launch_for_first_player(base: usize, max_players: u32) {
    const RVA_SET_SESSION_MAX_PLAYERS: usize = 0x38e888;
    const LAUNCH_STATE: usize = 0xc1a114;
    const STATE_START_MS: usize = 0xafba18;
    const LAUNCH_START_MS: usize = 0xafba1c;
    const RVA_ENGINE_MS: usize = 0x11a84;
    static WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let waiting = ((base + LAUNCH_STATE) as *const u32).read_volatile() == 8;
    if WAITING.swap(waiting, std::sync::atomic::Ordering::Relaxed) != waiting {
        log!("dedicated: {}", if waiting { "waiting for the first player (launch state 8)" } else { "first player joined, game starting" });
        if waiting && max_players > 0 {
            let set_max: unsafe extern "system" fn(u32) -> u8 = transmute(base + RVA_SET_SESSION_MAX_PLAYERS);
            log!("dedicated: session max players {max_players}: {}", if set_max(max_players) != 0 { "set" } else { "refused" });
        }
    }
    if waiting {
        let now: unsafe extern "system" fn() -> i32 = transmute(base + RVA_ENGINE_MS);
        let t = now();
        ((base + STATE_START_MS) as *mut i32).write_volatile(t);
        ((base + LAUNCH_START_MS) as *mut i32).write_volatile(t);
    }
}

/// Dedicated server: the UI's per-frame user check (`CheckLocalUsersSignedIn` 0x1802ab79c) leaves the session
/// (life-cycle state 5, exit reason 5) when no local user is signed in, unless its 500 ms grace timer
/// (`GetEngineMilliseconds` value at 0x1829f58bc) was refreshed recently. Keep refreshing it.
unsafe fn keep_session_without_local_users(base: usize) {
    const USER_CHECK_GRACE_MS: usize = 0x29f58bc;
    const RVA_ENGINE_MS: usize = 0x11a84;
    let now: unsafe extern "system" fn() -> i32 = transmute(base + RVA_ENGINE_MS);
    ((base + USER_CHECK_GRACE_MS) as *mut i32).write_volatile(now());
}

/// Every 10 s: process CPU time (user + kernel) over wall time, as % of one core.
struct CpuLog {
    start: std::time::Instant,
    last: std::time::Instant,
    last_cpu: u64,
}

impl CpuLog {
    fn new() -> Self {
        let now = std::time::Instant::now();
        CpuLog { start: now, last: now, last_cpu: process_cpu_100ns() }
    }
    fn tick(&mut self) {
        if self.last.elapsed() < std::time::Duration::from_secs(10) {
            return;
        }
        let cpu = process_cpu_100ns();
        let wall = self.last.elapsed().as_secs_f64();
        log!("cpu {:.0}s: {:.1}% of one core over the last {wall:.1} s, {:.1} s CPU total",
            self.start.elapsed().as_secs_f64(), (cpu - self.last_cpu) as f64 / 1e7 / wall * 100.0, cpu as f64 / 1e7);
        self.last = std::time::Instant::now();
        self.last_cpu = cpu;
    }
}

fn process_cpu_100ns() -> u64 {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_err() {
            return 0;
        }
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    ft(k) + ft(u)
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}

fn main() {
    // Per-monitor DPI aware before any window exists: otherwise Windows with display scaling above 100 % gives the
    // game a smaller window and stretches it up (blurry, low-detail picture). Wine ignores scaling, so it never showed.
    unsafe {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let argv: Vec<String> = std::env::args().collect();
    if let Some(i) = argv.iter().position(|a| a == "--browse") {
        // No engine, no log file: just list the servers.
        // No url (or the next flag): the default master list.
        let url = argv.get(i + 1).filter(|u| !u.starts_with("--")).cloned().unwrap_or_default();
        if let Err(e) = browser::browse(&url) {
            eprintln!("browse failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let log_name = if argv.iter().any(|a| a == "--preview") { "reach-host-preview.log" } else { "reach-host.log" };
    log::init(&exe_dir().join(log_name));
    if let Some(i) = argv.iter().position(|a| a == "--upnp-test") {
        // Add and remove one UDP mapping on the router, report what it said (upnp-test.txt; Proton drops stdout).
        let port = argv.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(47800);
        let r = upnp::self_test(port);
        let _ = std::fs::write(exe_dir().join("upnp-test.txt"), &r);
        println!("{r}");
        return;
    }
    if let Err(e) = run() {
        log!("FATAL: {e}");
        std::process::exit(1);
    }
    log!("done");
}
