//! Read-only diagnostic for the camera's extension units and standard controls.
//!
//! Usage:
//!   probe list            capture devices
//!   probe units           vendor nodes and which known XU GUIDs they answer to
//!   probe scan            dump every selector (GET_CUR) of every node/GUID; diff two runs
//!   probe ptz             pan/tilt/zoom and image-control ranges
//!   probe try-mode <id> [flag] --write
//!                         EXPERIMENT: writes selector 2 (byte0=id, byte1=flag), reads
//!                         back, prints every selector that changed, then restores.
//!
//!   probe poke <png-prefix> <offset> <hexbytes> [<offset> <hexbytes>...] --write
//!                         EXPERIMENT: patch bytes of selector 2, photograph, restore.
//!   probe snap <file.png>  save one preview frame (read-only)
//!
//! Every command except `try-mode ... --write` is read-only.

use linkcam_win::{list_cameras, ComGuard, KNOWN_XU_GUIDS};

/// UVC extension-unit selectors are 1..=32 in the descriptor bitmap; scan a bit beyond.
const MAX_SELECTOR: u8 = 0x30;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("")
}

fn main() {
    let _com = ComGuard::init();
    let command = std::env::args().nth(1).unwrap_or_else(|| "units".into());
    let result = match command.as_str() {
        "list" => list(),
        "units" => units(),
        "scan" => scan(),
        "ptz" => ptz(),
        "try-mode" => try_mode(),
        "snap" => snap(),
        "formats" => formats(),
        "mic" => mic(),
        "ptz-test" => ptz_test(),
        "poke" => poke(),
        "mode-sweep" => mode_sweep(),
        other => Err(format!("unknown command `{other}` (list|units|scan|ptz)")),
    };
    if let Err(message) = result {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn list() -> Result<(), String> {
    for camera in list_cameras().map_err(|e| e.to_string())? {
        println!("{}\t{}", camera.name, camera.device_path);
    }
    Ok(())
}

fn open() -> Result<linkcam_win::ProbeHandle, String> {
    linkcam_win::ProbeHandle::open().map_err(|e| e.to_string())
}

fn units() -> Result<(), String> {
    let handle = open()?;
    println!("camera: {} ({})", handle.info().name, handle.info().device_path);
    let nodes = handle.xu().extension_nodes();
    println!("vendor-specific nodes: {nodes:?}");
    for node in nodes {
        for guid in KNOWN_XU_GUIDS {
            let hits: Vec<u8> =
                (1..=MAX_SELECTOR).filter(|&s| handle.xu().access(node, &guid, s).is_some()).collect();
            if !hits.is_empty() {
                println!("node {node} guid {guid:?}: selectors {hits:02x?}");
            }
        }
    }
    Ok(())
}

fn scan() -> Result<(), String> {
    let handle = open()?;
    let xu = handle.xu();
    for node in xu.extension_nodes() {
        for guid in KNOWN_XU_GUIDS {
            for selector in 1..=MAX_SELECTOR {
                let Some(access) = xu.access(node, &guid, selector) else { continue };
                let (length, current) = match xu.read(node, &guid, selector) {
                    Ok(v) if access.readable => (v.len(), hex(&v)),
                    Ok(_) => (0, String::from("-")),
                    Err(e) => (0, format!("ERR({e})")),
                };
                println!(
                    "node={node} guid={guid:?} sel=0x{selector:02x} r={} w={} len={length} cur={current}",
                    access.readable as u8, access.writable as u8
                );
            }
        }
    }
    Ok(())
}

fn ptz() -> Result<(), String> {
    use linkcam_core::{CameraDevice, ImageControl};
    let mut camera = linkcam_win::WinCamera::open().map_err(|e| e.to_string())?;
    let limits = camera.limits();
    println!("pan  range {}..{}", limits.pan.min, limits.pan.max);
    println!("tilt range {}..{}", limits.tilt.min, limits.tilt.max);
    println!("zoom range {}..{}", limits.zoom.min, limits.zoom.max);
    match camera.read_state() {
        Ok(s) => println!(
            "state: pan={} tilt={} zoom={} mode={:?}",
            s.pan.raw(),
            s.tilt.raw(),
            s.zoom.raw(),
            s.mode
        ),
        Err(e) => println!("state: {e}"),
    }
    println!("mode selector reachable: {}", camera.supports_modes());
    for axis in [linkcam_win::PtzAxis::Pan, linkcam_win::PtzAxis::Tilt, linkcam_win::PtzAxis::Zoom] {
        match camera.ptz_control(axis) {
            Ok(v) => println!(
                "{:<5} value={} range={}..{} step={} default={} auto={}",
                axis.name(),
                v.value,
                v.min,
                v.max,
                v.step,
                v.default,
                v.auto
            ),
            Err(e) => println!("{:<5} {e}", axis.name()),
        }
    }
    for control in ImageControl::ALL {
        match camera.image_control(control) {
            Ok(v) => println!(
                "{:<14} value={} range={}..{} step={} default={} auto={}",
                control.label(),
                v.value,
                v.min,
                v.max,
                v.step,
                v.default,
                v.auto
            ),
            Err(e) => println!("{:<14} {e}", control.label()),
        }
    }
    Ok(())
}

type Snapshot = std::collections::BTreeMap<(u32, usize, u8), Vec<u8>>;

fn snapshot(xu: &linkcam_win::XuClient) -> Snapshot {
    let mut values = Snapshot::new();
    for node in xu.extension_nodes() {
        for (guid_index, guid) in KNOWN_XU_GUIDS.iter().enumerate() {
            for selector in 1..=MAX_SELECTOR {
                if xu.access(node, guid, selector).is_some_and(|a| a.readable) {
                    if let Ok(value) = xu.read(node, guid, selector) {
                        values.insert((node, guid_index, selector), value);
                    }
                }
            }
        }
    }
    values
}

fn print_changes(before: &Snapshot, after: &Snapshot) {
    for (key, new) in after {
        if before.get(key) != Some(new) {
            let old = before.get(key).map(|v| hex(v)).unwrap_or_default();
            println!(
                "  node={} guid#{} sel=0x{:02x}
    was {old}
    now {}",
                key.0,
                key.1,
                key.2,
                hex(new)
            );
        }
    }
}

fn try_mode() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    if !args.iter().any(|a| a == "--write") {
        return Err("try-mode writes to the camera; pass --write to confirm".into());
    }
    let numbers: Vec<u8> =
        args.iter().take_while(|a| !a.starts_with("--")).filter_map(|a| a.parse().ok()).collect();
    let mode = *numbers.first().ok_or("usage: probe try-mode <mode-id> [flag] --write")?;
    let flag = numbers.get(1).copied();

    let png_prefix = args.iter().position(|a| a == "--png").and_then(|i| args.get(i + 1)).cloned();
    let mut capture = match &png_prefix {
        Some(_) => {
            Some(linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    let mut photo = |label: &str| -> Result<(), String> {
        let (Some(prefix), Some(capture)) = (&png_prefix, capture.as_mut()) else { return Ok(()) };
        let mut last = None;
        for _ in 0..WARMUP_FRAMES {
            if let Some(f) = capture.next_frame().map_err(|e| e.to_string())? {
                last = Some(f);
            }
        }
        let frame = last.ok_or("no frame received")?;
        let path = format!("{prefix}-{label}.png");
        linkcam_win::preview::save_png(&frame, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
        println!("saved {path}");
        Ok(())
    };

    let handle = open()?;
    let xu = handle.xu();
    let guid = KNOWN_XU_GUIDS[0];
    let node = xu
        .extension_nodes()
        .into_iter()
        .find(|&n| xu.access(n, &guid, 2).is_some())
        .ok_or("mode selector not found")?;

    let original = xu.read(node, &guid, 2).map_err(|e| e.to_string())?;
    println!("original selector 2 ({} bytes): {}", original.len(), hex(&original));
    let before = snapshot(xu);

    let mut candidate = original.clone();
    candidate[0] = mode;
    if let Some(flag) = flag {
        candidate[1] = flag;
    }
    photo("before")?;
    println!("writing mode={mode} flag={flag:?}");
    xu.set(node, &guid, 2, &candidate).map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(800));

    let readback = xu.read(node, &guid, 2).map_err(|e| e.to_string())?;
    println!("readback: {}", hex(&readback));
    photo("during")?;
    let after = snapshot(xu);
    println!("selectors changed by the mode switch:");
    print_changes(&before, &after);

    xu.set(node, &guid, 2, &original).map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    let restored = xu.read(node, &guid, 2).map_err(|e| e.to_string())?;
    println!("restored: {} (matches original: {})", hex(&restored), restored == original);
    photo("restored")?;
    Ok(())
}

/// Frames discarded before saving so auto exposure/white balance can settle.
const WARMUP_FRAMES: usize = 20;

fn snap() -> Result<(), String> {
    let path = std::env::args().nth(2).ok_or("usage: probe snap <file.png>")?;
    let mut capture =
        linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?;
    let format = capture.format();
    let (w, h) = (format.width, format.height);
    println!("capturing {w}x{h}");
    let mut last = None;
    for _ in 0..WARMUP_FRAMES + 1 {
        if let Some(frame) = capture.next_frame().map_err(|e| e.to_string())? {
            last = Some(frame);
        }
    }
    let frame = last.ok_or("no frame received")?;
    linkcam_win::preview::save_png(&frame, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    println!("saved {path}");
    Ok(())
}

/// Sets zoom (and optionally pan/tilt) through the standard controls, photographs, then restores.
fn ptz_test() -> Result<(), String> {
    use linkcam_core::{AngleUnits, CameraDevice, ZoomUnits};
    let args: Vec<String> = std::env::args().skip(2).collect();
    let prefix = args.first().ok_or("usage: probe ptz-test <png-prefix> <zoom> [pan] [tilt]")?;
    let number = |i: usize| args.get(i).and_then(|v| v.parse::<i32>().ok());
    let mut camera = linkcam_win::WinCamera::open().map_err(|e| e.to_string())?;
    let mut capture =
        linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?;
    let mut photo = |label: &str| -> Result<(), String> {
        let mut last = None;
        for _ in 0..WARMUP_FRAMES {
            if let Some(f) = capture.next_frame().map_err(|e| e.to_string())? {
                last = Some(f);
            }
        }
        let frame = last.ok_or("no frame received")?;
        let path = format!("{prefix}-{label}.png");
        linkcam_win::preview::save_png(&frame, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
        println!("saved {path}");
        Ok(())
    };
    let original = camera.read_state().map_err(|e| e.to_string())?;
    println!(
        "original: pan={} tilt={} zoom={}",
        original.pan.raw(),
        original.tilt.raw(),
        original.zoom.raw()
    );
    photo("before")?;
    if let Some(zoom) = number(1) {
        let state = camera.set_zoom(ZoomUnits::from_raw(zoom as u16)).map_err(|e| e.to_string());
        println!("set zoom {zoom}: {state:?}");
    }
    if let (Some(pan), Some(tilt)) = (number(2), number(3)) {
        let state = camera
            .set_pan_tilt(AngleUnits::from_raw(pan), AngleUnits::from_raw(tilt))
            .map_err(|e| e.to_string());
        println!("set pan/tilt {pan}/{tilt}: {state:?}");
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    println!("now: {:?}", camera.read_state().map_err(|e| e.to_string())?);
    photo("during")?;
    let _ = camera.set_zoom(original.zoom);
    let _ = camera.set_pan_tilt(original.pan, original.tilt);
    Ok(())
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("odd-length hex `{text}`"));
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn poke() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(2).filter(|a| a != "--write").collect();
    if !std::env::args().any(|a| a == "--write") {
        return Err("poke writes to the camera; pass --write to confirm".into());
    }
    // Optional `--zoom N`: standard zoom control (restored afterwards) so pan/tilt have room to move.
    let zoom = args.iter().position(|a| a == "--zoom").map(|i| {
        let value = args.get(i + 1).and_then(|v| v.parse::<u16>().ok());
        args.drain(i..(i + 2).min(args.len()));
        value
    });
    // Optional `--sel 0x1a --node 1 --guid 0`: poke another selector (default: mode selector 2).
    let mut take = |flag: &str| {
        args.iter().position(|a| a == flag).map(|i| {
            let value = args.get(i + 1).cloned();
            args.drain(i..(i + 2).min(args.len()));
            value
        })
    };
    let selector = take("--sel")
        .flatten()
        .and_then(|v| u8::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or(2);
    let node_arg = take("--node").flatten().and_then(|v| v.parse::<u32>().ok());
    let guid_index = take("--guid").flatten().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let mut camera = linkcam_win::WinCamera::open().map_err(|e| e.to_string())?;
    let original_zoom = linkcam_core::CameraDevice::read_state(&mut camera).map_err(|e| e.to_string())?.zoom;
    if let Some(Some(zoom)) = zoom {
        linkcam_core::CameraDevice::set_zoom(&mut camera, linkcam_core::ZoomUnits::from_raw(zoom))
            .map_err(|e| e.to_string())?;
    }
    let prefix = args.first().ok_or("usage: probe poke <png-prefix> <offset> <hexbytes>... --write")?;
    let patches: Vec<(usize, Vec<u8>)> = args[1..]
        .chunks(2)
        .map(|pair| {
            let offset = pair[0].trim_start_matches("0x");
            let offset = usize::from_str_radix(offset, 16).map_err(|e| e.to_string())?;
            Ok((offset, parse_hex(pair.get(1).ok_or("missing hex bytes")?)?))
        })
        .collect::<Result<_, String>>()?;

    let mut capture =
        linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?;
    let handle = open()?;
    let xu = handle.xu();
    let guid = KNOWN_XU_GUIDS[guid_index];
    let node = node_arg
        .or_else(|| xu.extension_nodes().into_iter().find(|&n| xu.access(n, &guid, selector).is_some()))
        .ok_or("selector not found")?;

    let original = xu.read(node, &guid, selector).map_err(|e| e.to_string())?;
    let mut candidate = original.clone();
    for (offset, bytes) in &patches {
        candidate[*offset..*offset + bytes.len()].copy_from_slice(bytes);
    }
    println!("node={node} guid#{guid_index} sel=0x{selector:02x}");
    println!("original: {}", hex(&original));
    println!("patched : {}", hex(&candidate));
    xu.set(node, &guid, selector, &candidate).map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(900));
    println!("readback: {}", hex(&xu.read(node, &guid, selector).map_err(|e| e.to_string())?));
    let mut last = None;
    for _ in 0..WARMUP_FRAMES {
        if let Some(f) = capture.next_frame().map_err(|e| e.to_string())? {
            last = Some(f);
        }
    }
    let frame = last.ok_or("no frame received")?;
    let path = format!("{prefix}.png");
    linkcam_win::preview::save_png(&frame, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    println!("saved {path}");
    xu.set(node, &guid, selector, &original).map_err(|e| e.to_string())?;
    let _ = linkcam_core::CameraDevice::set_zoom(&mut camera, original_zoom);
    Ok(())
}

/// Writes every mode id 0..=15 with each flag in `FLAGS`, reporting which ones the
/// camera echoes back (valid) versus rejects (0xff). Restores the original payload.
fn mode_sweep() -> Result<(), String> {
    const FLAGS: [u8; 4] = [0x00, 0x01, 0x03, 0x10];
    if !std::env::args().any(|a| a == "--write") {
        return Err("mode-sweep writes to the camera; pass --write to confirm".into());
    }
    // Modes only apply while the camera is streaming.
    let mut capture =
        linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?;
    let _ = capture.next_frame();
    let handle = open()?;
    let xu = handle.xu();
    let guid = KNOWN_XU_GUIDS[0];
    let node = xu
        .extension_nodes()
        .into_iter()
        .find(|&n| xu.access(n, &guid, 2).is_some())
        .ok_or("no mode selector")?;
    let original = xu.read(node, &guid, 2).map_err(|e| e.to_string())?;
    for id in 0u8..=15 {
        for flag in FLAGS {
            let mut payload = original.clone();
            payload[0] = id;
            payload[1] = flag;
            if xu.set(node, &guid, 2, &payload).is_err() {
                println!("mode {id:2} flag {flag:#04x}: write rejected");
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(800));
            let back = xu.read(node, &guid, 2).map_err(|e| e.to_string())?;
            println!("mode {id:2} flag {flag:#04x}: readback {:02x} {:02x}", back[0], back[1]);
            // Return to normal between attempts so transitions never overlap.
            let mut reset = original.clone();
            reset[0] = 0;
            reset[1] = 0;
            let _ = xu.set(node, &guid, 2, &reset);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    let mut normal = original.clone();
    normal[0] = 0;
    normal[1] = 0;
    xu.set(node, &guid, 2, &normal).map_err(|e| e.to_string())?;
    Ok(())
}

fn formats() -> Result<(), String> {
    let capture = linkcam_win::preview::PreviewCapture::open("Insta360", None).map_err(|e| e.to_string())?;
    println!("running at {}", capture.format().label());
    for format in capture.available_formats() {
        println!("  {}x{} @ {} fps  ({})", format.width, format.height, format.fps, format.label());
    }
    Ok(())
}

/// Captures two seconds from the microphone and prints how much signal arrived.
fn mic() -> Result<(), String> {
    println!("microphones: {:?}", linkcam_win::audio::list_microphones());
    // No argument = automatic selection, like the app.
    let name = std::env::args().nth(2).unwrap_or_default();
    let capture = linkcam_win::audio::MicCapture::start(&name).map_err(|e| e.to_string())?;
    println!("format: {:?}", capture.format());
    std::thread::sleep(std::time::Duration::from_secs(2));
    let samples = capture.drain();
    let rms = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt();
    println!("{} samples, rms {rms:.1}", samples.len());
    Ok(())
}
