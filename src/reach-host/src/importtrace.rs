//! `REACH_IMPORT_TRACE=1`: what a game really uses from Windows, for the native Linux server study
//! (notes/native-linux.md). Every import slot of haloreach.dll goes through a counting stub
//! (`mov rax, counter; lock inc [rax]; mov rax, real; jmp rax`: no argument register touched), runtime
//! LoadLibrary / GetProcAddress lookups are logged and their results counted the same way, and the D3D11 device,
//! immediate context and swap chain vtables are counted per method. `import-trace.txt` next to the exe is rewritten
//! every 30 s. Off by default; nothing here runs without the variable.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use windows::core::PCSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::*;

use crate::log;

pub fn enabled() -> bool {
    std::env::var_os("REACH_IMPORT_TRACE").is_some_and(|v| v != "0")
}

struct Entry {
    /// "KERNEL32.dll!CreateFileW", "dyn:xaudio2_9.dll!XAudio2Create", "d3d11ctx!Draw".
    name: String,
    count: &'static AtomicU64,
    /// Return address of the latest call (the stub stores it), sampled by the dump thread for hot entries.
    last: &'static AtomicU64,
    stub: usize,
}

static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// Executable stub memory: current page and bytes used in it.
static PAGE: Mutex<(usize, usize)> = Mutex::new((0, 0));
static LOOKUPS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A counting stub for `name` that jumps to `real` (one per name: a reloaded engine reuses it).
unsafe fn stub(name: String, real: usize) -> usize {
    let mut e = ENTRIES.lock().unwrap();
    if let Some(x) = e.iter().find(|x| x.name == name) {
        return x.stub;
    }
    let count: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
    let last: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
    let mut p = PAGE.lock().unwrap();
    if p.0 == 0 || p.1 + 64 > 0x10000 {
        let m = VirtualAlloc(None, 0x10000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
        if m.is_null() {
            return real;
        }
        *p = (m as usize, 0);
    }
    let at = p.0 + p.1;
    p.1 += 64;
    let mut b = Vec::with_capacity(64);
    b.extend([0x48, 0x8B, 0x04, 0x24]); // mov rax, [rsp] (return address)
    b.extend([0x49, 0xBB]); // mov r11, &last (r11: volatile, never an argument)
    b.extend((last as *const AtomicU64 as u64).to_le_bytes());
    b.extend([0x49, 0x89, 0x03]); // mov [r11], rax
    b.extend([0x48, 0xB8]);
    b.extend((count as *const AtomicU64 as u64).to_le_bytes());
    b.extend([0xF0, 0x48, 0xFF, 0x00]);
    b.extend([0x48, 0xB8]);
    b.extend((real as u64).to_le_bytes());
    b.extend([0xFF, 0xE0]);
    std::ptr::copy_nonoverlapping(b.as_ptr(), at as *mut u8, b.len());
    e.push(Entry { name, count, last, stub: at });
    at
}

fn executable(addr: usize) -> bool {
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    unsafe {
        if VirtualQuery(Some(addr as *const c_void), &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
            return false;
        }
    }
    let p = mbi.Protect.0;
    p & (PAGE_EXECUTE.0 | PAGE_EXECUTE_READ.0 | PAGE_EXECUTE_READWRITE.0 | PAGE_EXECUTE_WRITECOPY.0) != 0
}

unsafe fn write_slot(slot: *mut usize, v: usize) {
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(slot as _, 8, PAGE_READWRITE, &mut old).is_ok() {
        slot.write(v);
        let _ = VirtualProtect(slot as _, 8, old, &mut old);
    }
}

unsafe fn cstr(p: usize) -> String {
    std::ffi::CStr::from_ptr(p as *const i8).to_string_lossy().into_owned()
}

// ---- runtime lookups ------------------------------------------------------------------------------------------

static REAL_GPA: AtomicUsize = AtomicUsize::new(0);
static REAL_LLA: AtomicUsize = AtomicUsize::new(0);
static REAL_LLW: AtomicUsize = AtomicUsize::new(0);
static REAL_LLEXA: AtomicUsize = AtomicUsize::new(0);
static REAL_LLEXW: AtomicUsize = AtomicUsize::new(0);
static MODULE_NAMES: Mutex<Option<HashMap<usize, String>>> = Mutex::new(None);

fn note(s: String) {
    let mut l = LOOKUPS.lock().unwrap();
    if !l.contains(&s) {
        log!("import-trace: {s}");
        l.push(s);
    }
}

fn note_module(h: usize, name: String) {
    if h != 0 {
        MODULE_NAMES.lock().unwrap().get_or_insert_with(HashMap::new).insert(h, name.clone());
    }
    note(format!("LoadLibrary {name} -> {}", if h == 0 { "failed" } else { "ok" }));
}

unsafe fn wstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

unsafe extern "system" fn hook_lla(n: *const i8) -> usize {
    let r: unsafe extern "system" fn(*const i8) -> usize = std::mem::transmute(REAL_LLA.load(Ordering::Relaxed));
    let h = r(n);
    note_module(h, cstr(n as usize));
    h
}
unsafe extern "system" fn hook_llw(n: *const u16) -> usize {
    let r: unsafe extern "system" fn(*const u16) -> usize = std::mem::transmute(REAL_LLW.load(Ordering::Relaxed));
    let h = r(n);
    note_module(h, wstr(n));
    h
}
unsafe extern "system" fn hook_llexa(n: *const i8, f: usize, fl: u32) -> usize {
    let r: unsafe extern "system" fn(*const i8, usize, u32) -> usize = std::mem::transmute(REAL_LLEXA.load(Ordering::Relaxed));
    let h = r(n, f, fl);
    note_module(h, cstr(n as usize));
    h
}
unsafe extern "system" fn hook_llexw(n: *const u16, f: usize, fl: u32) -> usize {
    let r: unsafe extern "system" fn(*const u16, usize, u32) -> usize = std::mem::transmute(REAL_LLEXW.load(Ordering::Relaxed));
    let h = r(n, f, fl);
    note_module(h, wstr(n));
    h
}

static REAL_RAISE: AtomicUsize = AtomicUsize::new(0);
static REAL_COCREATE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn hook_raise(code: u32, flags: u32, n: u32, args: *const usize) {
    let extra = if code == 0x406D1388 && n >= 2 && !args.is_null() { format!(" (thread name {:?})", cstr(*args.add(1))) } else { String::new() };
    note(format!("RaiseException {code:#x} flags {flags:#x} args {n}{extra}"));
    let r: unsafe extern "system" fn(u32, u32, u32, *const usize) = std::mem::transmute(REAL_RAISE.load(Ordering::Relaxed));
    r(code, flags, n, args)
}

unsafe extern "system" fn hook_cocreate(clsid: *const [u8; 16], outer: usize, ctx: u32, iid: *const [u8; 16], out: usize) -> i32 {
    let g = |p: *const [u8; 16]| {
        let b = *p;
        format!("{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{}", u32::from_le_bytes([b[0], b[1], b[2], b[3]]), u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]), b[8], b[9], b[10..].iter().map(|x| format!("{x:02x}")).collect::<String>())
    };
    let r: unsafe extern "system" fn(*const [u8; 16], usize, u32, *const [u8; 16], usize) -> i32 = std::mem::transmute(REAL_COCREATE.load(Ordering::Relaxed));
    let hr = r(clsid, outer, ctx, iid, out);
    note(format!("CoCreateInstance clsid {} iid {} -> {hr:#x}", g(clsid), g(iid)));
    hr
}

static REAL_REFLECT: AtomicUsize = AtomicUsize::new(0);

const REFLECTION: &[&str] = &["QueryInterface", "AddRef", "Release", "GetDesc", "GetConstantBufferByIndex", "GetConstantBufferByName",
    "GetResourceBindingDesc", "GetInputParameterDesc", "GetOutputParameterDesc", "GetPatchConstantParameterDesc", "GetVariableByName",
    "GetResourceBindingDescByName", "GetMovInstructionCount", "GetMovcInstructionCount", "GetConversionInstructionCount",
    "GetBitwiseInstructionCount", "GetGSInputPrimitive", "IsSampleFrequencyShader", "GetNumInterfaceSlots", "GetMinFeatureLevel",
    "GetThreadGroupSize", "GetRequiresFlags"];
/// ID3D11ShaderReflectionConstantBuffer (no IUnknown).
const REFLECTION_CB: &[&str] = &["GetDesc", "GetVariableByIndex", "GetVariableByName"];
/// ID3D11ShaderReflectionVariable.
const REFLECTION_VAR: &[&str] = &["GetDesc", "GetType", "GetBuffer", "GetInterfaceSlot"];

/// Count calls on each method of the vtable at `vt` (once per vtable).
unsafe fn count_vtable(vt: *mut usize, tag: &str, names: &[&str]) {
    if vt.is_null() || ENTRIES.lock().unwrap().iter().any(|e| e.stub == vt.read()) {
        return;
    }
    for (i, m) in names.iter().enumerate() {
        let slot = vt.add(i);
        write_slot(slot, stub(format!("{tag}!{m}"), slot.read()));
    }
}

static REAL_GET_CB: AtomicUsize = AtomicUsize::new(0);
static REAL_CB_GET_VAR: AtomicUsize = AtomicUsize::new(0);

/// ID3D11ShaderReflection::GetConstantBufferByIndex: also count the constant buffer object's methods.
unsafe extern "system" fn hook_get_cb(this: usize, i: u32) -> usize {
    let r: unsafe extern "system" fn(usize, u32) -> usize = std::mem::transmute(REAL_GET_CB.load(Ordering::Relaxed));
    let cb = r(this, i);
    if cb != 0 {
        let vt = *(cb as *const *mut usize);
        if REAL_CB_GET_VAR.load(Ordering::Relaxed) == 0 {
            REAL_CB_GET_VAR.store(vt.add(1).read(), Ordering::Relaxed);
            count_vtable(vt, "reflect-cb", REFLECTION_CB);
            write_slot(vt.add(1), stub("reflect-cb!GetVariableByIndex(wrapped)".into(), hook_cb_get_var as *const () as usize));
        }
    }
    cb
}

unsafe extern "system" fn hook_cb_get_var(this: usize, i: u32) -> usize {
    let r: unsafe extern "system" fn(usize, u32) -> usize = std::mem::transmute(REAL_CB_GET_VAR.load(Ordering::Relaxed));
    let v = r(this, i);
    if v != 0 {
        count_vtable(*(v as *const *mut usize), "reflect-var", REFLECTION_VAR);
    }
    v
}

/// D3DReflect: count the methods the engine calls on the shader reflection objects it gets.
unsafe extern "system" fn hook_reflect(src: usize, size: usize, iid: usize, out: *mut usize) -> i32 {
    let r: unsafe extern "system" fn(usize, usize, usize, *mut usize) -> i32 = std::mem::transmute(REAL_REFLECT.load(Ordering::Relaxed));
    let hr = r(src, size, iid, out);
    if hr == 0 && !out.is_null() && *out != 0 {
        let vt = *(*out as *const *mut usize);
        if REAL_GET_CB.load(Ordering::Relaxed) == 0 {
            REAL_GET_CB.store(vt.add(4).read(), Ordering::Relaxed);
            count_vtable(vt, "reflect", REFLECTION);
            write_slot(vt.add(4), stub("reflect!GetConstantBufferByIndex(wrapped)".into(), hook_get_cb as *const () as usize));
        }
    }
    hr
}

/// GetProcAddress: log the name and hand back a counting stub for it.
unsafe extern "system" fn hook_gpa(m: usize, name: usize) -> usize {
    let r: unsafe extern "system" fn(usize, usize) -> usize = std::mem::transmute(REAL_GPA.load(Ordering::Relaxed));
    let f = r(m, name);
    let module = MODULE_NAMES.lock().unwrap().as_ref().and_then(|x| x.get(&m).cloned()).unwrap_or_else(|| module_name(m));
    let sym = if name >> 16 == 0 { format!("#{name}") } else { cstr(name) };
    note(format!("GetProcAddress {module}!{sym} -> {}", if f == 0 { "missing" } else { "ok" }));
    if f == 0 || !executable(f) {
        return f;
    }
    if sym == "D3DReflect" {
        REAL_REFLECT.store(f, Ordering::Relaxed);
        return stub(format!("dyn:{module}!{sym}"), hook_reflect as *const () as usize);
    }
    stub(format!("dyn:{module}!{sym}"), f)
}

fn module_name(m: usize) -> String {
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    let mut b = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(Some(HMODULE(m as _)), &mut b) } as usize;
    let s = String::from_utf16_lossy(&b[..n]);
    s.rsplit(['\\', '/']).next().unwrap_or("?").to_string()
}

// ---- import table ---------------------------------------------------------------------------------------------

/// Route every import slot of the module at `base` through a counting stub (and the lookup functions through
/// the logging hooks above). Call after the other import hooks (input.rs), so they stay in the chain.
pub unsafe fn install(base: usize) {
    if !enabled() {
        return;
    }
    let rd32 = |a: usize| (a as *const u32).read_unaligned() as usize;
    let pe = base + rd32(base + 0x3c);
    let opt = pe + 24;
    let (imp_rva, _) = (rd32(opt + 112 + 8), rd32(opt + 112 + 12)); // data directory 1: imports
    let k32 = GetModuleHandleW(windows::core::w!("kernel32.dll")).unwrap_or_default();
    let real = |n: &[u8]| GetProcAddress(k32, PCSTR(n.as_ptr())).map(|f| f as usize).unwrap_or(0);
    let ole = GetModuleHandleW(windows::core::w!("ole32.dll")).unwrap_or_default();
    let co_real = GetProcAddress(ole, PCSTR(b"CoCreateInstance\0".as_ptr())).map(|f| f as usize).unwrap_or(0);
    let specials: [(&[u8], usize, &AtomicUsize); 6] = [
        (b"RaiseException\0", hook_raise as *const () as usize, &REAL_RAISE),
        (b"GetProcAddress\0", hook_gpa as *const () as usize, &REAL_GPA),
        (b"LoadLibraryA\0", hook_lla as *const () as usize, &REAL_LLA),
        (b"LoadLibraryW\0", hook_llw as *const () as usize, &REAL_LLW),
        (b"LoadLibraryExA\0", hook_llexa as *const () as usize, &REAL_LLEXA),
        (b"LoadLibraryExW\0", hook_llexw as *const () as usize, &REAL_LLEXW),
    ];
    let (mut n, mut skipped) = (0, 0);
    let mut d = base + imp_rva;
    while rd32(d + 12) != 0 {
        let dll = cstr(base + rd32(d + 12));
        let (oft, ft) = (rd32(d), rd32(d + 16));
        let mut i = 0;
        loop {
            let ent = ((base + if oft != 0 { oft } else { ft } + i * 8) as *const u64).read();
            if ent == 0 {
                break;
            }
            let sym = if ent >> 63 != 0 { format!("#{}", ent & 0xffff) } else { cstr(base + (ent as usize & 0x7fff_ffff) + 2) };
            let slot = (base + ft + i * 8) as *mut usize;
            let cur = slot.read();
            i += 1;
            // Data imports (C++ globals such as std::cout) are not code: leave them.
            if !executable(cur) {
                skipped += 1;
                continue;
            }
            let special = specials.iter().find(|s| dll.eq_ignore_ascii_case("KERNEL32.dll") && s.0[..s.0.len() - 1] == *sym.as_bytes());
            let target = if dll.eq_ignore_ascii_case("ole32.dll") && sym == "CoCreateInstance" && cur == co_real && co_real != 0 {
                REAL_COCREATE.store(cur, Ordering::Relaxed);
                hook_cocreate as *const () as usize
            } else if let Some((nm, hook, store)) = special {
                if cur != real(nm) {
                    log!("import-trace: {sym} slot already hooked, counting only");
                    cur
                } else {
                    store.store(cur, Ordering::Relaxed);
                    *hook
                }
            } else {
                cur
            };
            write_slot(slot, stub(format!("{dll}!{sym}"), target));
            n += 1;
        }
        d += 20;
    }
    log!("import-trace: {n} import slots counted, {skipped} data imports left alone");
    start_dump();
}

// ---- D3D11 ----------------------------------------------------------------------------------------------------

const DEVICE: &[&str] = &["QueryInterface", "AddRef", "Release", "CreateBuffer", "CreateTexture1D", "CreateTexture2D", "CreateTexture3D",
    "CreateShaderResourceView", "CreateUnorderedAccessView", "CreateRenderTargetView", "CreateDepthStencilView", "CreateInputLayout",
    "CreateVertexShader", "CreateGeometryShader", "CreateGeometryShaderWithStreamOutput", "CreatePixelShader", "CreateHullShader",
    "CreateDomainShader", "CreateComputeShader", "CreateClassLinkage", "CreateBlendState", "CreateDepthStencilState",
    "CreateRasterizerState", "CreateSamplerState", "CreateQuery", "CreatePredicate", "CreateCounter", "CreateDeferredContext",
    "OpenSharedResource", "CheckFormatSupport", "CheckMultisampleQualityLevels", "CheckCounterInfo", "CheckCounter",
    "CheckFeatureSupport", "GetPrivateData", "SetPrivateData", "SetPrivateDataInterface", "GetFeatureLevel", "GetCreationFlags",
    "GetDeviceRemovedReason", "GetImmediateContext", "SetExceptionMode", "GetExceptionMode"];

const CONTEXT: &[&str] = &["QueryInterface", "AddRef", "Release", "GetDevice", "GetPrivateData", "SetPrivateData", "SetPrivateDataInterface",
    "VSSetConstantBuffers", "PSSetShaderResources", "PSSetShader", "PSSetSamplers", "VSSetShader", "DrawIndexed", "Draw", "Map", "Unmap",
    "PSSetConstantBuffers", "IASetInputLayout", "IASetVertexBuffers", "IASetIndexBuffer", "DrawIndexedInstanced", "DrawInstanced",
    "GSSetConstantBuffers", "GSSetShader", "IASetPrimitiveTopology", "VSSetShaderResources", "VSSetSamplers", "Begin", "End", "GetData",
    "SetPredication", "GSSetShaderResources", "GSSetSamplers", "OMSetRenderTargets", "OMSetRenderTargetsAndUnorderedAccessViews",
    "OMSetBlendState", "OMSetDepthStencilState", "SOSetTargets", "DrawAuto", "DrawIndexedInstancedIndirect", "DrawInstancedIndirect",
    "Dispatch", "DispatchIndirect", "RSSetState", "RSSetViewports", "RSSetScissorRects", "CopySubresourceRegion", "CopyResource",
    "UpdateSubresource", "CopyStructureCount", "ClearRenderTargetView", "ClearUnorderedAccessViewUint", "ClearUnorderedAccessViewFloat",
    "ClearDepthStencilView", "GenerateMips", "SetResourceMinLOD", "GetResourceMinLOD", "ResolveSubresource", "ExecuteCommandList",
    "HSSetShaderResources", "HSSetShader", "HSSetSamplers", "HSSetConstantBuffers", "DSSetShaderResources", "DSSetShader", "DSSetSamplers",
    "DSSetConstantBuffers", "CSSetShaderResources", "CSSetUnorderedAccessViews", "CSSetShader", "CSSetSamplers", "CSSetConstantBuffers",
    "VSGetConstantBuffers", "PSGetShaderResources", "PSGetShader", "PSGetSamplers", "VSGetShader", "PSGetConstantBuffers",
    "IAGetInputLayout", "IAGetVertexBuffers", "IAGetIndexBuffer", "GSGetConstantBuffers", "GSGetShader", "IAGetPrimitiveTopology",
    "VSGetShaderResources", "VSGetSamplers", "GetPredication", "GSGetShaderResources", "GSGetSamplers", "OMGetRenderTargets",
    "OMGetRenderTargetsAndUnorderedAccessViews", "OMGetBlendState", "OMGetDepthStencilState", "SOGetTargets", "RSGetState",
    "RSGetViewports", "RSGetScissorRects", "HSGetShaderResources", "HSGetShader", "HSGetSamplers", "HSGetConstantBuffers",
    "DSGetShaderResources", "DSGetShader", "DSGetSamplers", "DSGetConstantBuffers", "CSGetShaderResources", "CSGetUnorderedAccessViews",
    "CSGetShader", "CSGetSamplers", "CSGetConstantBuffers", "ClearState", "Flush", "GetType", "GetContextFlags", "FinishCommandList"];

const SWAPCHAIN: &[&str] = &["QueryInterface", "AddRef", "Release", "SetPrivateData", "SetPrivateDataInterface", "GetPrivateData", "GetParent",
    "GetDevice", "Present", "GetBuffer", "SetFullscreenState", "GetFullscreenState", "GetDesc", "ResizeBuffers", "ResizeTarget",
    "GetContainingOutput", "GetFrameStatistics", "GetLastPresentCount", "GetDesc1", "GetFullscreenDesc", "GetHwnd", "GetCoreWindow",
    "Present1", "IsTemporaryMonoSupported", "GetRestrictToOutput", "SetBackgroundColor", "GetBackgroundColor", "SetRotation", "GetRotation"];

/// Count calls per method on the vtables of these COM objects (shared by every object of the same class, so our
/// own UI's calls count too; a dedicated server has none).
pub unsafe fn install_d3d(device: *mut c_void, context: *mut c_void, swapchain: *mut c_void) {
    if !enabled() {
        return;
    }
    for (obj, tag, names) in [(device, "d3d11dev", DEVICE), (context, "d3d11ctx", CONTEXT), (swapchain, "dxgiswap", SWAPCHAIN)] {
        let vt = *(obj as *const *mut usize);
        for (i, m) in names.iter().enumerate() {
            let slot = vt.add(i);
            let cur = slot.read();
            if ENTRIES.lock().unwrap().iter().any(|e| e.stub == cur) {
                continue; // already counted (a later game in this process)
            }
            write_slot(slot, stub(format!("{tag}!{m}"), cur));
        }
    }
    log!("import-trace: D3D11 device / context / swap chain methods counted");
    start_dump();
}

// ---- report ---------------------------------------------------------------------------------------------------

fn start_dump() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| {
            let mut t = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(2));
                sample();
                if t.elapsed().as_secs() >= 30 {
                    t = std::time::Instant::now();
                    dump();
                }
            }
        });
    });
}

/// Per hot entry (over 100k calls): how often each return address was the latest caller at a sample.
static CALLERS: Mutex<Option<HashMap<String, HashMap<u64, u32>>>> = Mutex::new(None);

fn sample() {
    let e = ENTRIES.lock().unwrap();
    let mut c = CALLERS.lock().unwrap();
    let c = c.get_or_insert_with(HashMap::new);
    for x in e.iter().filter(|x| x.count.load(Ordering::Relaxed) > 100_000) {
        let a = x.last.load(Ordering::Relaxed);
        *c.entry(x.name.clone()).or_default().entry(a).or_default() += 1;
    }
}

/// "haloreach+0x1234" for addresses in the engine, else the raw address.
fn where_is(a: u64) -> String {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed) as u64;
    if base != 0 && a >= base && a < base + 0x3000000 { format!("haloreach+{:#x}", a - base) } else { format!("{a:#x}") }
}

pub fn dump() {
    let e = ENTRIES.lock().unwrap();
    let mut used: Vec<(u64, &str)> = e.iter().map(|x| (x.count.load(Ordering::Relaxed), x.name.as_str())).filter(|x| x.0 > 0).collect();
    used.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let unused: Vec<&str> = e.iter().filter(|x| x.count.load(Ordering::Relaxed) == 0).map(|x| x.name.as_str()).collect();
    let mut out = format!("# import trace: {} entries, {} called, {} never called\n\n## called (count, name)\n", e.len(), used.len(), unused.len());
    for (c, n) in &used {
        out += &format!("{c:>12}  {n}\n");
    }
    out += "\n## never called\n";
    for n in &unused {
        out += &format!("{n}\n");
    }
    out += "\n## hot callers (sampled every 2 ms: latest return address)\n";
    if let Some(c) = CALLERS.lock().unwrap().as_ref() {
        let mut names: Vec<&String> = c.keys().collect();
        names.sort();
        for n in names {
            let mut v: Vec<(&u64, &u32)> = c[n].iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            let total: u32 = v.iter().map(|x| *x.1).sum();
            out += &format!("{n}: {}\n", v.iter().take(4).map(|(a, k)| format!("{} {}%", where_is(**a), **k * 100 / total.max(1))).collect::<Vec<_>>().join(", "));
        }
    }
    out += "\n## runtime lookups\n";
    for l in LOOKUPS.lock().unwrap().iter() {
        out += &format!("{l}\n");
    }
    drop(e);
    let p = std::env::current_exe().unwrap_or_default().with_file_name("import-trace.txt");
    let _ = std::fs::write(p, out);
}
