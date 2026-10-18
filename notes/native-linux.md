# Native Linux dedicated server (study, 2026-10-18)

Goal: run `haloreach.dll` on Linux without Wine/Proton, in the way bbhost runs Bloodborne's PS4 executable. The
engine's x86-64 code runs as it is; we load the DLL ourselves and answer its Windows imports with our own code.
First target is the **dedicated server only**: it needs no window, no input, no sound and (with `--no-render`) almost
no graphics. Payoff: a Docker server without Wine, Xvfb, DXVK or a GPU.

## Measuring: `REACH_IMPORT_TRACE=1` (src/reach-host/src/importtrace.rs)

Every import slot of haloreach.dll goes through a counting stub (`mov rax, counter; lock inc [rax]; mov rax,
real; jmp rax`, so no argument register changes), runtime LoadLibrary / GetProcAddress lookups are logged and their
results counted too, and the D3D11 device, immediate context and swap chain vtables are counted per method.
`import-trace.txt` next to the exe is rewritten every 30 s (called with counts, never called, runtime lookups).
Installed after `LoadLibrary` returns, so **calls from DllMain / CRT start-up are not counted** (they show as
"never called": `_initterm`, `_beginthreadex`, `_initialize_onexit_table` ...).

Runs (hidden, silent): `REACH_IMPORT_TRACE=1 RUN=native CLIENTS=1 scripts/run-dedicated.sh --hidden
[--no-render] --game-variant <gun-game.bin>`, one client playing ~2 min (`work/trace-play.txt`) to game over.
Results kept in `work/native-trace-render.txt` and `work/native-trace-norender.txt`.

## Results

378 static import slots. During a game the server calls **about 180 Windows functions** (plus start-up ones):

| Module | called | never | Notes |
|---|---|---|---|
| KERNEL32 | 80 | 43 | threads, CS/events/semaphores, SList, files, time, heap, **fibers** (CreateFiberEx 379, SwitchToFiber 8.5k), TLS |
| UCRT (api-ms-win-crt-*) | ~63 | ~66 | math (sqrtf, sinf, powf ...), strings incl. wide, `__stdio_common_*` printf/scanf family, qsort, time |
| VCRUNTIME140 | 11 | 10 | mem*/str*; `__C_specific_handler` 12 and `RaiseException` 12 (SEH runs; probably the MSVC thread-name exception, to confirm) |
| MSVCP140 | 0 | 7 | mutex helpers only, never called in game |
| bcrypt | 8 | 2 | hashing (BCryptHashData 5k) + BCryptVerifySignature 5 (content signatures) |
| WS2_32 | 8 | 23 | by ordinal: select (#18, 9.8k), socket, setsockopt, ntohl/ntohs, WSAStartup |
| USER32 | 5 | 18 | GetFocus, EnumWindows, GetWindowThreadProcessId ... (window lookups) |
| WINMM | 3 | 17 | timeGetTime, timeBeginPeriod, waveOutGetNumDevs |
| ole32 | 2 | 6 | CoInitializeEx, CoCreateInstance ×4 (CLSIDs to log) |
| ADVAPI32 | 1 | 5 | GetUserNameA |

Runtime lookups: `dxgi!CreateDXGIFactory1`, `D3DCOMPILER_47!D3DCompile/D3DReflect/D3DGetInputSignatureBlob`
(D3DReflect 5.2k: the engine reflects its shaders while loading), `xaudio2_9redist!XAudio2Create (#1)` +
X3DAudio, `bink2w64` (BinkOpenXAudio2 at game end), `kernel32!GetLogicalProcessorInformation`.
No C++ exception was thrown (`_CxxThrowException` never called).

### Graphics

| | normal server (320x180) | `--no-render` |
|---|---|---|
| context methods called | 38 (Draw 207k, Map/Unmap 614k ...) | 10, nearly all once; CopySubresourceRegion 11k (texture uploads), ClearState 410 |
| device methods called | 21 | 15: resource and shader **creation** while a map loads (CreateBuffer 16k, CreateTexture2D 4k, CreatePixelShader 4k, CreateQuery 5k, views) |
| swap chain | Present 8.7k | Present 410 (loading screen only) |

So with `--no-render` a **null D3D11 device** is enough: COM objects with win64 vtables that hand out dummy
textures, buffers, views, shaders and queries and ignore the rest. Unknown: which `ID3D11ShaderReflection`
methods the engine reads after D3DReflect (needs a probe on that object's vtable); our stub would report no
resources. The `--no-render` render thread spins (notes/dedicated.md); natively we pace it ourselves.

### Probes (second run, `--no-render`)

- **RaiseException** is only ever 0x406D1388, MSVC's "set thread name" (12 threads: MAIN_THREAD, job0-2,
  ASYNC_0-2, NET_DEBUG, BLOCK_DETECT, RENDER, NETWORK_RECEIVE, AUDIO). Natively: a no-op (or pthread_setname_np);
  no SEH dispatcher needed for normal play.
- **CoCreateInstance** only creates MMDeviceEnumerator (CLSID bcde0395-…, IID IMMDeviceEnumerator a95664d2-…),
  for audio devices; the first try fails (CO_E_NOTINITIALIZED) and is retried. Natively: a null enumerator with
  one fake output device, or failure if the engine tolerates it (to check).
- **QueryPerformanceCounter ~900k calls/s** comes from a stopwatch helper (haloreach+0xab9d4: adds now - start to
  a timer struct; 68 % of samples) and +0x11ab8 (20 %). Profiling timers, not a busy-wait; cheap with a vDSO
  clock. Server CPU in that run: 23-33 % of one core.
- `import-trace.txt` now ends with "hot callers": per function over 100k calls, the return addresses seen most
  (the stub stores its caller; a thread samples every 2 ms).

### GPU-less server under Wine today: lavapipe (2026-10-18)

The dedicated server runs with **no GPU at all** on Mesa's software Vulkan driver (lavapipe) under DXVK:
`VK_ICD_FILENAMES=VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`, `--no-render`. DXVK's log
(`DXVK_LOG_LEVEL=info DXVK_LOG_PATH=...`) showed only `llvmpipe` and nvidia-smi listed no game process. One
client joined, spawned and played; server CPU 28-32 % of one core, the same as on the GPU (23-33 %), because
`--no-render` leaves only map-load resource creation and the throttle now paces the render thread (the old
"render thread spins at 100 %" note predates throttle.rs). So the Docker image could drop the NVIDIA requirement
with no native work: mesa-vulkan-drivers + the ICD variables + `--no-render`. Xvfb stays (Wine's window).
Done in public/docker (GPU=cpu default). Container test: ~38 % of one core in game, engine RSS ~2 GB (lavapipe's
device memory is shmem: ~1.4 GB of textures that a GPU would hold), so the memory limit is 4g. Debian's ICD file
is `lvp_icd.json` (no arch suffix); a wrong path gives DXVK "Required instance extensions not supported".

### Shader reflection

Server alone: 6 D3DReflect calls (the map loads only when the first player joins). With one client:
D3DReflect 5,246, then per shader GetDesc, GetResourceBindingDesc (66k) and GetConstantBufferByIndex (23k) +
the constant buffer's GetDesc. Never: variables, types, input/output signatures (D3DGetInputSignatureBlob is used
instead), instruction counts. A native stub needs the RDEF chunk of the DXBC blob (constant buffer names/sizes,
resource bindings), a small documented format, or may report empty shaders on a no-render server (untested).

## Plan (server only)

1. **PE loader** (new `reach-native` binary, Linux): map haloreach.dll, relocations, imports resolved to our
   `extern "win64"` functions (every unimplemented one gets a trap that logs its name), static TLS directory
   (`.tls` present) and a per-thread fake TEB behind GS (`arch_prctl(ARCH_SET_GS)`; compiled code reads
   gs:[0x30]/[0x58]), run DllMain and the CRT start-up.
2. **Win32 slice:** the ~180 functions above plus start-up ones: threads, sync, SList, fibers (own context
   switch), heap, files (Z:\ path mapping), time, wide strings, MS printf semantics, bcrypt via ring/sha.
   `RaiseException` for the thread-name code as a no-op; real SEH only if a probe shows it is needed.
3. **Null graphics + audio:** null D3D11 device / DXGI factory / swap chain, D3DCompile/D3DReflect stubs, null
   XAudio2 (voices accept buffers and report them played), Bink stub.
4. **Host port:** the server parts of reach-host (net, query, master, lobby, vote, playlist, throttle, stats) build
   for Linux; window, UI, input, audio stay Windows-only (cfg). Hooks use mprotect instead of VirtualProtect.
5. **Docker image** without Wine/Xvfb/DXVK/GPU; compare CPU and RAM with the Wine server; then a player test.

Still to check before step 1: start-up calls (install the stubs before DllMain: load with
DONT_RESOLVE_DLL_REFERENCES or patch from a LoadLibrary notification).
