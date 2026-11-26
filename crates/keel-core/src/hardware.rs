//! Decides where a local model should run, in ordinary terms.
//!
//! The CPU build of the pinned runner (catalog key `linux-x86_64` and the
//! other platform keys) ships only CPU libraries such as `libggml-cpu-*.so`.
//! It lists no graphics devices, so it cannot keep a model in graphics memory.
//! Graphics offload needs a build that has `libggml-vulkan.so`, `libggml-cuda.so`,
//! or `libggml-metal.so` (or the `.dll` / `.dylib` equivalent) beside the
//! executable, and `--list-devices` must name a real device. The Linux graphics
//! build from the same pinned release is catalog key `linux-x86_64-vulkan`.
//!
//! Launch flags (`--device`, `--gpu-layers`, `--fit`) are chosen here and are
//! not shown in Settings.

use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SMALL_MODEL_BYTES: u64 = 1_099_627_808;
const FULL_MARGIN_MIB: u64 = 256;
/// Below this free MiB, Auto/Graphics placement falls back to the processor.
pub const MIN_USEFUL_FREE_MIB: u64 = 512;
/// A discrete graphics chip at least this large can comfortably run the 4B model.
const DISCRETE_4B_MIB: u64 = 3072;
/// A discrete graphics chip at least this large can comfortably run the 9B model.
const DISCRETE_9B_MIB: u64 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunWhere {
    Auto,
    Graphics,
    Processor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsShare {
    Auto,
    Full,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Integrated,
    Discrete,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub physical_cores: u32,
    pub logical_cpus: u32,
    pub memory_bytes: u64,
    pub has_intel_graphics: bool,
    pub has_amd_graphics: bool,
    pub has_nvidia_graphics: bool,
    pub nvidia_driver_usable: bool,
    pub nvidia_memory_mib: u64,
    /// Free MiB from nvidia-smi, when reported.
    pub nvidia_free_mib: Option<u64>,
}

impl Machine {
    pub fn graphics_chip(&self) -> bool {
        self.has_intel_graphics || self.has_amd_graphics || self.has_nvidia_graphics
    }

    fn small_cpu(&self) -> bool {
        self.physical_cores <= 4 || self.logical_cpus <= 8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsDevice {
    pub id: String,
    pub name: String,
    pub total_mib: u64,
    pub free_mib: u64,
    pub kind: DeviceKind,
}

impl GraphicsDevice {
    /// Cap reported free memory to the device capacity.
    pub fn usable_mib(&self) -> u64 {
        self.free_mib.min(self.total_mib)
    }
}

/// Where the usable-free figure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventorySource {
    ListDevices,
    NvidiaSmi,
    None,
}

/// Snapshot used by runtime start, keep-alive, and pack unload decisions.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VramInventory {
    /// Best free MiB available for a new load (preferred device).
    pub usable_free_mib: u64,
    pub total_mib: u64,
    /// `"discrete"`, `"integrated"`, or empty when unknown.
    pub kind: String,
    pub source: InventorySource,
    /// Preferred graphics device when one is known.
    #[serde(skip)]
    pub preferred: Option<GraphicsDevice>,
    #[serde(skip)]
    pub devices: Vec<GraphicsDevice>,
}

impl VramInventory {
    pub fn empty() -> Self {
        Self {
            usable_free_mib: 0,
            total_mib: 0,
            kind: String::new(),
            source: InventorySource::None,
            preferred: None,
            devices: Vec::new(),
        }
    }

    /// True when the preferred device has at least `need_mib` free.
    pub fn has_free(&self, need_mib: u64) -> bool {
        need_mib == 0 || self.usable_free_mib >= need_mib
    }
}

/// What Settings and the runner need. Launch flags are omitted from the UI payload.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ComputerAdvice {
    pub summary: String,
    pub recommended_model_id: String,
    pub graphics_ready: bool,
    /// Best known graphics memory (MiB). Prefers `--list-devices`; falls back to nvidia-smi.
    pub vram_total_mib: u64,
    pub vram_free_mib: u64,
    /// `"discrete"`, `"integrated"`, or empty when unknown.
    pub vram_kind: String,
    /// Empty when using graphics or the person chose the processor; otherwise why.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub processor_reason: String,
    #[serde(skip)]
    pub args: Vec<String>,
    #[serde(skip)]
    pub use_graphics: bool,
}

pub fn canonical_run_where(value: &str) -> String {
    match parse_run_where(value) {
        RunWhere::Graphics => "graphics",
        RunWhere::Processor => "processor",
        RunWhere::Auto => "auto",
    }
    .to_string()
}

pub fn canonical_graphics_share(value: &str) -> String {
    match parse_graphics_share(value) {
        GraphicsShare::Full => "full",
        GraphicsShare::Partial => "partial",
        GraphicsShare::Auto => "auto",
    }
    .to_string()
}

pub fn parse_run_where(value: &str) -> RunWhere {
    match value.trim().to_ascii_lowercase().as_str() {
        "graphics" => RunWhere::Graphics,
        "processor" => RunWhere::Processor,
        _ => RunWhere::Auto,
    }
}

pub fn parse_graphics_share(value: &str) -> GraphicsShare {
    match value.trim().to_ascii_lowercase().as_str() {
        "full" => GraphicsShare::Full,
        "partial" => GraphicsShare::Partial,
        _ => GraphicsShare::Auto,
    }
}

/// True when Automatic or Graphics should look for a graphics-capable runner.
pub fn should_prepare_graphics(
    run_where: &str,
    graphics_chip: bool,
    already_installed: bool,
) -> bool {
    match parse_run_where(run_where) {
        RunWhere::Processor => false,
        RunWhere::Graphics => true,
        RunWhere::Auto => graphics_chip || already_installed,
    }
}

pub fn parse_cpuinfo(text: &str) -> (u32, u32) {
    let mut logical = 0u32;
    let mut cores_per_socket = 0u32;
    let mut sockets = BTreeSet::new();
    let mut saw_socket = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "processor" => logical += 1,
            "cpu cores" => cores_per_socket = value.parse().unwrap_or(0),
            "physical id" => {
                saw_socket = true;
                sockets.insert(value.to_string());
            }
            _ => {}
        }
    }
    let physical = if saw_socket && cores_per_socket > 0 {
        cores_per_socket.saturating_mul(sockets.len().max(1) as u32)
    } else if cores_per_socket > 0 {
        cores_per_socket
    } else {
        logical
    };
    (physical, logical)
}

pub fn parse_lspci(text: &str) -> (bool, bool, bool) {
    let mut intel = false;
    let mut amd = false;
    let mut nvidia = false;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let display = lower.contains("vga compatible controller")
            || lower.contains("3d controller")
            || lower.contains("display controller");
        if !display {
            continue;
        }
        if lower.contains("intel") {
            intel = true;
        }
        if lower.contains("nvidia") {
            nvidia = true;
        }
        if lower.contains("amd")
            || lower.contains("ati ")
            || lower.contains("advanced micro devices")
        {
            amd = true;
        }
    }
    (intel, amd, nvidia)
}

/// `(driver responded, largest total MiB, free MiB for that GPU)`.
/// Missing free memory stays unknown. A reported zero is a real measurement.
pub fn parse_nvidia_smi(text: &str) -> (bool, u64, Option<u64>) {
    let mut any = false;
    let mut best_total = 0u64;
    let mut best_free = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.to_ascii_lowercase().contains("failed") {
            continue;
        }
        any = true;
        let mut parts = line.split(',');
        let _name = parts.next();
        let total = parts.next().and_then(parse_mib_token).unwrap_or(0);
        let free = parts.next().and_then(parse_mib_token);
        if total >= best_total {
            best_total = total;
            best_free = free;
        }
    }
    (any, best_total, best_free)
}

/// Discrete NVIDIA chip inferred from nvidia-smi when `--list-devices` is empty.
pub fn nvidia_fallback_devices(machine: &Machine) -> Vec<GraphicsDevice> {
    if !machine.nvidia_driver_usable || machine.nvidia_memory_mib == 0 {
        return Vec::new();
    }
    let free = machine.nvidia_free_mib.unwrap_or(0);
    vec![GraphicsDevice {
        id: "nvidia".to_string(),
        name: "NVIDIA".to_string(),
        total_mib: machine.nvidia_memory_mib,
        free_mib: free,
        kind: DeviceKind::Discrete,
    }]
}

fn kind_label(kind: DeviceKind) -> String {
    match kind {
        DeviceKind::Discrete => "discrete".to_string(),
        DeviceKind::Integrated => "integrated".to_string(),
        DeviceKind::Software => String::new(),
    }
}

fn prefer_inventory_device(devices: &[GraphicsDevice]) -> Option<&GraphicsDevice> {
    devices.iter().max_by_key(|device| {
        (
            u8::from(device.kind == DeviceKind::Discrete),
            device.usable_mib(),
            device.total_mib,
        )
    })
}

/// Build the VRAM inventory used by start / keep-alive / pack scheduling.
/// Prefers probed `--list-devices`; falls back to nvidia-smi totals.
pub fn inventory(machine: &Machine, probed: &[GraphicsDevice]) -> VramInventory {
    let real: Vec<GraphicsDevice> = probed
        .iter()
        .filter(|device| device.kind != DeviceKind::Software)
        .cloned()
        .collect();
    let (devices, source) = if !real.is_empty() {
        (real, InventorySource::ListDevices)
    } else {
        let fallback = nvidia_fallback_devices(machine);
        if fallback.is_empty() {
            return VramInventory::empty();
        }
        (fallback, InventorySource::NvidiaSmi)
    };
    let preferred = prefer_inventory_device(&devices).cloned();
    match preferred {
        Some(device) => VramInventory {
            usable_free_mib: device.usable_mib(),
            total_mib: device.total_mib,
            kind: kind_label(device.kind),
            source,
            preferred: Some(device),
            devices,
        },
        None => VramInventory::empty(),
    }
}

/// Devices for model sizing: prefer probed `--list-devices`, else nvidia-smi.
fn sizing_devices(machine: &Machine, devices: &[GraphicsDevice]) -> Vec<GraphicsDevice> {
    inventory(machine, devices).devices
}

pub fn parse_device_list(text: &str) -> Vec<GraphicsDevice> {
    text.lines().filter_map(parse_device_line).collect()
}

fn parse_device_line(line: &str) -> Option<GraphicsDevice> {
    let line = line.trim().trim_start_matches('-').trim();
    let (head, tail) = line.rsplit_once(" (")?;
    let tail = tail.trim_end_matches(')');
    let mut parts = tail.split(',');
    let total = parse_mib_token(parts.next()?)?;
    let free = parse_mib_token(parts.next()?)?;
    if !tail.to_ascii_lowercase().contains("mib") {
        return None;
    }
    let (id, name) = head.split_once(':')?;
    let id = id.trim().to_string();
    let name = name.trim().to_string();
    if id.is_empty() || name.is_empty() {
        return None;
    }
    let kind = classify_device(&id, &name);
    Some(GraphicsDevice {
        id,
        name,
        total_mib: total,
        free_mib: free,
        kind,
    })
}

fn parse_mib_token(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse().ok()
}

fn classify_device(id: &str, name: &str) -> DeviceKind {
    let id = id.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    if id == "cpu"
        || id.starts_with("cpu")
        || name.contains("llvmpipe")
        || name.contains("swiftshader")
    {
        return DeviceKind::Software;
    }
    if id.starts_with("cuda")
        || name.contains("nvidia")
        || name.contains("geforce")
        || name.contains("quadro")
        || name.contains("radeon rx")
        || name.contains("radeon pro")
        || name.contains("arc")
    {
        return DeviceKind::Discrete;
    }
    if name.contains("uhd")
        || name.contains("iris")
        || name.contains("hd graphics")
        || name.contains("radeon graphics")
        || name.contains("vega")
        || name.contains("intel")
    {
        return DeviceKind::Integrated;
    }
    DeviceKind::Integrated
}

/// Weights plus a cushion for a short conversation fit in `device_mib`.
pub fn model_fits_entirely(device_mib: u64, model_bytes: u64) -> bool {
    if device_mib == 0 || model_bytes == 0 {
        return false;
    }
    let model_mib = model_bytes.div_ceil(1024 * 1024);
    device_mib >= model_mib + model_mib / 4 + FULL_MARGIN_MIB
}

pub fn recommended_model_id(machine: &Machine, devices: &[GraphicsDevice]) -> &'static str {
    let real = sizing_devices(machine, devices);
    if let Some(discrete) = real
        .iter()
        .filter(|device| device.kind == DeviceKind::Discrete)
        .map(|device| device.total_mib.max(device.free_mib))
        .max()
    {
        if discrete >= DISCRETE_9B_MIB {
            return "bonsai-9b";
        }
        if discrete >= DISCRETE_4B_MIB {
            return "bonsai-4b";
        }
        // A small laptop graphics chip (about 2 GB) stays on the fast small model,
        // even when integrated graphics also report a large shared heap.
        return "lfm3-1.7b";
    }
    if real
        .iter()
        .any(|device| device.kind == DeviceKind::Integrated)
    {
        let memory = real
            .iter()
            .map(|device| device.total_mib.max(device.free_mib))
            .max()
            .unwrap_or(0);
        if memory >= DISCRETE_4B_MIB || machine.memory_bytes >= 8 * 1024 * 1024 * 1024 {
            return "bonsai-4b";
        }
        return "lfm3-1.7b";
    }
    cpu_recommendation(machine)
}

pub fn lighter_model_id(recommended: &str) -> Option<&'static str> {
    match recommended {
        "bonsai-9b" => Some("bonsai-4b"),
        "bonsai-4b" => Some("lfm3-1.7b"),
        _ => None,
    }
}

fn cpu_recommendation(machine: &Machine) -> &'static str {
    if machine.memory_bytes < 8 * 1024 * 1024 * 1024 {
        return "lfm3-1.7b";
    }
    if !machine.small_cpu()
        && machine.physical_cores >= 8
        && machine.memory_bytes >= 32 * 1024 * 1024 * 1024
    {
        return "bonsai-9b";
    }
    if !machine.small_cpu() && machine.memory_bytes >= 12 * 1024 * 1024 * 1024 {
        return "bonsai-4b";
    }
    "lfm3-1.7b"
}

pub fn advise(
    machine: &Machine,
    run_where: &str,
    graphics_share: &str,
    model_bytes: u64,
    devices: &[GraphicsDevice],
) -> ComputerAdvice {
    let model_bytes = if model_bytes == 0 {
        SMALL_MODEL_BYTES
    } else {
        model_bytes
    };
    let recommended = recommended_model_id(machine, devices).to_string();
    let snap = inventory(machine, devices);
    // Launch offload still needs a probed device id from `--list-devices`.
    // nvidia-smi only informs recommendation and the VRAM inventory above.
    let graphics_ready = devices
        .iter()
        .any(|device| device.kind != DeviceKind::Software);
    let preference = parse_run_where(run_where);
    let share_pref = parse_graphics_share(graphics_share);
    let (device, insufficient_free) = pick_device(devices, model_bytes);
    let wants_graphics = matches!(preference, RunWhere::Auto | RunWhere::Graphics);
    let use_graphics = device.is_some() && wants_graphics;
    let processor_reason =
        if !use_graphics && preference != RunWhere::Processor && insufficient_free {
            format!(
            "Not enough free graphics memory (need about {MIN_USEFUL_FREE_MIB} MiB; {} MiB free).",
            snap.usable_free_mib
        )
        } else {
            String::new()
        };
    let (args, whole) = if let (true, Some(device)) = (use_graphics, device) {
        let fits = model_fits_entirely(device.usable_mib(), model_bytes);
        let whole = match share_pref {
            GraphicsShare::Full => true,
            GraphicsShare::Partial => false,
            GraphicsShare::Auto => fits,
        };
        (graphics_args(device, whole, model_bytes), whole)
    } else {
        (processor_args(), false)
    };
    let summary = summary_sentence(
        preference,
        machine.graphics_chip(),
        graphics_ready,
        use_graphics,
        whole,
        &processor_reason,
    );
    ComputerAdvice {
        summary,
        recommended_model_id: recommended,
        graphics_ready,
        vram_total_mib: snap.total_mib,
        vram_free_mib: snap.usable_free_mib,
        vram_kind: snap.kind,
        processor_reason,
        args,
        use_graphics,
    }
}

/// Pick a probed device with enough free VRAM. Returns `(device, insufficient_free)`.
/// `insufficient_free` is true when real devices exist but none have useful free memory.
pub fn pick_device(
    devices: &[GraphicsDevice],
    model_bytes: u64,
) -> (Option<&GraphicsDevice>, bool) {
    let real: Vec<&GraphicsDevice> = devices
        .iter()
        .filter(|device| device.kind != DeviceKind::Software)
        .collect();
    if real.is_empty() {
        return (None, false);
    }
    let fitting: Vec<&GraphicsDevice> = real
        .iter()
        .copied()
        .filter(|device| model_fits_entirely(device.usable_mib(), model_bytes))
        .collect();
    if !fitting.is_empty() {
        let best = fitting.into_iter().max_by_key(|device| {
            (
                u8::from(device.kind == DeviceKind::Discrete),
                device.usable_mib(),
            )
        });
        return (best, false);
    }
    let partial: Vec<&GraphicsDevice> = real
        .iter()
        .copied()
        .filter(|device| device.usable_mib() >= MIN_USEFUL_FREE_MIB)
        .collect();
    if let Some(best) = partial.into_iter().max_by_key(|device| device.usable_mib()) {
        return (Some(best), false);
    }
    (None, true)
}

fn graphics_args(device: &GraphicsDevice, whole: bool, model_bytes: u64) -> Vec<String> {
    let mut args = vec![
        "--device".to_string(),
        device.id.clone(),
        "--split-mode".to_string(),
        "none".to_string(),
    ];
    if whole {
        // Every layer the runner will accept. `--fit off` keeps it from
        // quietly moving part of the model back onto the processor.
        args.extend([
            "--gpu-layers".into(),
            "all".into(),
            "--fit".into(),
            "off".into(),
        ]);
    } else {
        args.extend([
            "--gpu-layers".into(),
            "auto".into(),
            "--fit".into(),
            "on".into(),
            "--fit-target".into(),
            partial_fit_target(device.usable_mib(), model_bytes).to_string(),
        ]);
    }
    args
}

fn processor_args() -> Vec<String> {
    vec![
        "--device".into(),
        "none".into(),
        "--gpu-layers".into(),
        "0".into(),
        "--fit".into(),
        "off".into(),
    ]
}

/// Leave about half the weights on the processor by raising the free-memory margin.
fn partial_fit_target(device_mib: u64, model_bytes: u64) -> u64 {
    let model_mib = model_bytes.div_ceil(1024 * 1024);
    let keep = (model_mib / 2).max(128);
    device_mib.saturating_sub(keep).max(256)
}

fn summary_sentence(
    preference: RunWhere,
    graphics_chip: bool,
    graphics_ready: bool,
    use_graphics: bool,
    whole: bool,
    processor_reason: &str,
) -> String {
    if use_graphics {
        return match (preference, whole) {
            (RunWhere::Auto, true) => {
                "Automatic keeps the whole model on the graphics chip.".to_string()
            }
            (RunWhere::Auto, false) => {
                "Automatic puts part of the model on the graphics chip. The rest stays on the processor.".to_string()
            }
            (_, true) => "The whole model sits on the graphics chip.".to_string(),
            (_, false) => {
                "Part of the model sits on the graphics chip. The rest stays on the processor."
                    .to_string()
            }
        };
    }
    if !processor_reason.is_empty() {
        return format!("{processor_reason} The model runs on the processor.");
    }
    match preference {
        RunWhere::Processor => "The model runs on the processor.".to_string(),
        RunWhere::Graphics => {
            "The graphics chip isn't ready yet, so the model runs on the processor.".to_string()
        }
        RunWhere::Auto if graphics_chip && !graphics_ready => {
            "This computer has a graphics chip. Automatic will use it when chat can run there. Right now the model runs on the processor.".to_string()
        }
        RunWhere::Auto => "Automatic runs the model on the processor.".to_string(),
    }
}

/// The sidecar library that means this executable can offload to graphics memory.
pub fn graphics_library_present(bin: &Path) -> bool {
    let Some(dir) = bin.parent() else {
        return false;
    };
    const NAMES: &[&str] = &[
        "libggml-vulkan.so",
        "libggml-cuda.so",
        "libggml-metal.so",
        "ggml-vulkan.dll",
        "ggml-cuda.dll",
        "ggml-metal.dll",
        "libggml-vulkan.dylib",
        "libggml-metal.dylib",
    ];
    NAMES.iter().any(|name| dir.join(name).is_file())
}

pub fn probe_devices(bin: &Path) -> Vec<GraphicsDevice> {
    if !graphics_library_present(bin) {
        return Vec::new();
    }
    let Some(text) = run_capture(bin, &["--list-devices"], Duration::from_secs(5)) else {
        return Vec::new();
    };
    parse_device_list(&text)
        .into_iter()
        .filter(|device| device.kind != DeviceKind::Software)
        .collect()
}

pub fn detect_machine() -> Machine {
    let (mut physical, mut logical) =
        parse_cpuinfo(&std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default());
    if logical == 0 {
        let fallback = std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(1)
            .max(1);
        logical = fallback;
        physical = fallback;
    }
    if physical == 0 {
        physical = logical;
    }
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let pci = run_capture(Path::new("lspci"), &["-nn"], Duration::from_secs(2)).unwrap_or_default();
    let (has_intel_graphics, has_amd_graphics, has_nvidia_graphics) = parse_lspci(&pci);
    let nvidia = run_capture(
        Path::new("nvidia-smi"),
        &[
            "--query-gpu=name,memory.total,memory.free",
            "--format=csv,noheader",
        ],
        Duration::from_secs(2),
    );
    let (nvidia_driver_usable, nvidia_memory_mib, nvidia_free_mib) = nvidia
        .as_deref()
        .map(parse_nvidia_smi)
        .unwrap_or((false, 0, None));
    Machine {
        physical_cores: physical,
        logical_cpus: logical,
        memory_bytes: system.total_memory(),
        has_intel_graphics,
        has_amd_graphics,
        has_nvidia_graphics,
        nvidia_driver_usable,
        nvidia_memory_mib,
        nvidia_free_mib,
    }
}

fn run_capture(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(program);
    crate::runtime::apply_std_runner_env(&mut cmd, program);
    let mut child = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut out);
                }
                return Some(out);
            }
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(_) => return None,
        }
    }
}

/// Kept so a caller can point at a graphics build that is not the managed folder.
pub fn graphics_binary(path: Option<&str>) -> Option<PathBuf> {
    let path = path.filter(|value| !value.trim().is_empty())?;
    let path = PathBuf::from(path.trim());
    (path.is_file() && graphics_library_present(&path)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LFM3_BYTES: u64 = 1_099_627_808;
    const BONSAI_9B_BYTES: u64 = 2_556_640_128;
    const DEVICE_LIST: &str = "\
Available devices:
  Vulkan0: Intel(R) UHD Graphics 620 (WHL GT2) (11842 MiB, 9537 MiB free)
  Vulkan1: NVIDIA GeForce MX150 (NVK GP108) (2304 MiB, 2073 MiB free)
";

    fn zenbook() -> Machine {
        Machine {
            physical_cores: 4,
            logical_cpus: 8,
            memory_bytes: 15 * 1024 * 1024 * 1024,
            has_intel_graphics: true,
            has_amd_graphics: false,
            has_nvidia_graphics: true,
            nvidia_driver_usable: false,
            nvidia_memory_mib: 0,
            nvidia_free_mib: None,
        }
    }

    fn devices() -> Vec<GraphicsDevice> {
        parse_device_list(DEVICE_LIST)
    }

    fn has_flag(args: &[String], name: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == name && pair[1] == value)
    }

    fn assert_plain(text: &str) {
        let lower = text.to_ascii_lowercase();
        for word in [
            "llama", "gguf", "backend", "vulkan", "cuda", "uhd", "api key", "620",
        ] {
            assert!(!lower.contains(word), "{text}");
        }
    }

    #[test]
    fn parses_cpu_pci_nvidia_and_device_list() {
        let cpu = "\
processor\t: 0
model name\t: Intel(R) Core(TM) i7-8565U CPU @ 1.80GHz
physical id\t: 0
cpu cores\t: 4
siblings\t: 8
processor\t: 1
cpu cores\t: 4
physical id\t: 0
";
        assert_eq!(parse_cpuinfo(cpu), (4, 2));
        let pci = "\
00:02.0 VGA compatible controller [0300]: Intel Corporation WhiskeyLake-U GT2 [UHD Graphics 620] [8086:3ea0]
02:00.0 3D controller [0302]: NVIDIA Corporation GP108M [GeForce MX150] [10de:1d12]
";
        assert_eq!(parse_lspci(pci), (true, false, true));
        assert_eq!(
            parse_lspci("00:1f.0 ISA bridge: Intel Corporation"),
            (false, false, false)
        );
        assert_eq!(
            parse_nvidia_smi("NVIDIA GeForce MX150, 2048 MiB\n"),
            (true, 2048, None)
        );
        assert_eq!(
            parse_nvidia_smi("NVIDIA GeForce RTX 4070, 12288 MiB, 11000 MiB\n"),
            (true, 12288, Some(11000))
        );
        assert_eq!(parse_nvidia_smi(""), (false, 0, None));
        let found = devices();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "Vulkan0");
        assert_eq!(found[0].kind, DeviceKind::Integrated);
        assert_eq!(found[0].free_mib, 9537);
        assert_eq!(found[1].id, "Vulkan1");
        assert_eq!(found[1].kind, DeviceKind::Discrete);
        assert_eq!(found[1].total_mib, 2304);
        let software = parse_device_list(
            "  Vulkan0: llvmpipe (LLVM 18.1.0, 256 bits) (16384 MiB, 16000 MiB free)\n",
        );
        assert_eq!(software[0].kind, DeviceKind::Software);
    }

    #[test]
    fn small_cpu_without_graphics_stays_on_the_fast_model() {
        let advice = advise(&zenbook(), "auto", "auto", LFM3_BYTES, &[]);
        assert_eq!(advice.recommended_model_id, "lfm3-1.7b");
        assert!(!advice.use_graphics);
        assert_eq!(
            advice.args,
            vec!["--device", "none", "--gpu-layers", "0", "--fit", "off"]
        );
        assert!(advice.summary.contains("processor"));
        assert_plain(&advice.summary);
        assert!(lighter_model_id("lfm3-1.7b").is_none());
    }

    #[test]
    fn zenbook_keeps_the_small_model_fully_on_dedicated_graphics_memory() {
        let found = devices();
        assert!(model_fits_entirely(2073, LFM3_BYTES));
        let advice = advise(&zenbook(), "auto", "auto", LFM3_BYTES, &found);
        assert_eq!(advice.recommended_model_id, "lfm3-1.7b");
        assert!(advice.use_graphics);
        // Prefer the discrete MX150 inventory over the inflated UHD shared heap.
        assert_eq!(advice.vram_total_mib, 2304);
        assert_eq!(advice.vram_free_mib, 2073);
        assert_eq!(advice.vram_kind, "discrete");
        assert_eq!(
            advice.args,
            vec![
                "--device",
                "Vulkan1",
                "--split-mode",
                "none",
                "--gpu-layers",
                "all",
                "--fit",
                "off"
            ]
        );
        assert_eq!(
            advice.summary,
            "Automatic keeps the whole model on the graphics chip."
        );
        assert_plain(&advice.summary);
    }

    #[test]
    fn a_model_too_big_for_the_small_chip_uses_the_integrated_chip() {
        let advice = advise(&zenbook(), "auto", "auto", BONSAI_9B_BYTES, &devices());
        assert!(has_flag(&advice.args, "--device", "Vulkan0"));
        assert!(has_flag(&advice.args, "--gpu-layers", "all"));
    }

    #[test]
    fn partial_share_leaves_room_on_the_graphics_chip() {
        let advice = advise(&zenbook(), "graphics", "partial", LFM3_BYTES, &devices());
        assert!(has_flag(&advice.args, "--device", "Vulkan1"));
        assert!(has_flag(&advice.args, "--gpu-layers", "auto"));
        assert!(has_flag(&advice.args, "--fit", "on"));
        assert!(advice.args.iter().any(|arg| arg == "--fit-target"));
        assert!(!advice.args.iter().any(|arg| arg == "all"));
        assert_plain(&advice.summary);
    }

    #[test]
    fn processor_choice_ignores_a_working_graphics_chip() {
        let advice = advise(&zenbook(), "processor", "full", LFM3_BYTES, &devices());
        assert!(!advice.use_graphics);
        assert_eq!(advice.args[1], "none");
        assert_eq!(advice.summary, "The model runs on the processor.");
    }

    #[test]
    fn graphics_choice_without_a_device_stays_on_the_processor() {
        let advice = advise(&zenbook(), "graphics", "full", LFM3_BYTES, &[]);
        assert!(!advice.graphics_ready);
        assert!(!advice.use_graphics);
        assert!(advice.summary.contains("isn't ready"));
        assert_plain(&advice.summary);
    }

    #[test]
    fn a_mini_pc_with_integrated_graphics_uses_vulkan_and_a_slightly_larger_model() {
        let machine = Machine {
            physical_cores: 4,
            logical_cpus: 4,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            has_intel_graphics: true,
            has_amd_graphics: false,
            has_nvidia_graphics: false,
            nvidia_driver_usable: false,
            nvidia_memory_mib: 0,
            nvidia_free_mib: None,
        };
        let devices = parse_device_list("  Vulkan0: Intel(R) Graphics (8192 MiB, 7000 MiB free)\n");
        assert_eq!(devices[0].kind, DeviceKind::Integrated);
        assert_eq!(recommended_model_id(&machine, &devices), "bonsai-4b");
        assert_eq!(lighter_model_id("bonsai-4b"), Some("lfm3-1.7b"));
        let advice = advise(&machine, "auto", "auto", LFM3_BYTES, &devices);
        assert_eq!(advice.vram_kind, "integrated");
        assert_eq!(advice.vram_total_mib, 8192);
        assert!(has_flag(&advice.args, "--device", "Vulkan0"));
        assert!(has_flag(&advice.args, "--gpu-layers", "all"));
        assert_plain(&advice.summary);
    }

    #[test]
    fn a_large_discrete_chip_can_default_to_the_smarter_model() {
        let machine = Machine {
            physical_cores: 8,
            logical_cpus: 16,
            memory_bytes: 32 * 1024 * 1024 * 1024,
            has_intel_graphics: false,
            has_amd_graphics: false,
            has_nvidia_graphics: true,
            nvidia_driver_usable: true,
            nvidia_memory_mib: 12288,
            nvidia_free_mib: Some(11000),
        };
        let devices =
            parse_device_list("  CUDA0: NVIDIA GeForce RTX 4070 (12288 MiB, 11000 MiB free)\n");
        assert_eq!(devices[0].kind, DeviceKind::Discrete);
        assert_eq!(recommended_model_id(&machine, &devices), "bonsai-9b");
        assert_eq!(lighter_model_id("bonsai-9b"), Some("bonsai-4b"));
        let advice = advise(&machine, "auto", "auto", BONSAI_9B_BYTES, &devices);
        assert_eq!(advice.vram_total_mib, 12288);
        assert_eq!(advice.vram_kind, "discrete");
        assert!(has_flag(&advice.args, "--device", "CUDA0"));
        assert!(has_flag(&advice.args, "--gpu-layers", "all"));
    }

    #[test]
    fn nvidia_smi_sizes_the_model_when_list_devices_is_empty() {
        let machine = Machine {
            physical_cores: 8,
            logical_cpus: 16,
            memory_bytes: 32 * 1024 * 1024 * 1024,
            has_intel_graphics: false,
            has_amd_graphics: false,
            has_nvidia_graphics: true,
            nvidia_driver_usable: true,
            nvidia_memory_mib: 12288,
            nvidia_free_mib: Some(11000),
        };
        assert_eq!(recommended_model_id(&machine, &[]), "bonsai-9b");
        let advice = advise(&machine, "auto", "auto", BONSAI_9B_BYTES, &[]);
        assert_eq!(advice.recommended_model_id, "bonsai-9b");
        assert_eq!(advice.vram_total_mib, 12288);
        assert_eq!(advice.vram_free_mib, 11000);
        assert_eq!(advice.vram_kind, "discrete");
        // No probed device id → stay on the processor for launch flags.
        assert!(!advice.graphics_ready);
        assert!(!advice.use_graphics);
        assert_eq!(advice.args[1], "none");
    }

    #[test]
    fn small_nvidia_without_devices_stays_on_the_fast_model() {
        let machine = Machine {
            physical_cores: 4,
            logical_cpus: 8,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            has_intel_graphics: true,
            has_amd_graphics: false,
            has_nvidia_graphics: true,
            nvidia_driver_usable: true,
            nvidia_memory_mib: 2048,
            nvidia_free_mib: Some(1800),
        };
        // Same discrete-threshold rule as a probed MX150: do not let shared
        // system RAM inflate the recommendation when VRAM is only ~2 GB.
        assert_eq!(recommended_model_id(&machine, &[]), "lfm3-1.7b");
        let advice = advise(&machine, "auto", "auto", LFM3_BYTES, &[]);
        assert_eq!(advice.vram_total_mib, 2048);
        assert_eq!(advice.vram_kind, "discrete");
        assert!(!advice.use_graphics);
    }

    #[test]
    fn a_strong_processor_with_lots_of_memory_can_use_the_smarter_model() {
        let machine = Machine {
            physical_cores: 16,
            logical_cpus: 32,
            memory_bytes: 64 * 1024 * 1024 * 1024,
            has_intel_graphics: false,
            has_amd_graphics: false,
            has_nvidia_graphics: false,
            nvidia_driver_usable: false,
            nvidia_memory_mib: 0,
            nvidia_free_mib: None,
        };
        assert_eq!(recommended_model_id(&machine, &[]), "bonsai-9b");
        let modest = Machine {
            physical_cores: 6,
            logical_cpus: 12,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            ..machine
        };
        assert_eq!(recommended_model_id(&modest, &[]), "bonsai-4b");
    }

    #[test]
    fn unknown_preferences_stay_on_automatic() {
        assert_eq!(canonical_run_where(""), "auto");
        assert_eq!(canonical_run_where("nope"), "auto");
        assert_eq!(canonical_graphics_share("full"), "full");
        assert!(!should_prepare_graphics("processor", true, true));
        assert!(should_prepare_graphics("auto", true, false));
        assert!(!should_prepare_graphics("auto", false, false));
    }

    #[test]
    fn inventory_prefers_discrete_free_memory_from_list_devices() {
        let snap = inventory(&zenbook(), &devices());
        assert_eq!(snap.source, InventorySource::ListDevices);
        assert_eq!(snap.kind, "discrete");
        assert_eq!(snap.total_mib, 2304);
        assert_eq!(snap.usable_free_mib, 2073);
        assert_eq!(
            snap.preferred.as_ref().map(|d| d.id.as_str()),
            Some("Vulkan1")
        );
        assert!(snap.has_free(2000));
        assert!(!snap.has_free(3000));
    }

    #[test]
    fn inventory_falls_back_to_nvidia_smi_when_list_devices_is_empty() {
        let machine = Machine {
            nvidia_driver_usable: true,
            nvidia_memory_mib: 12288,
            nvidia_free_mib: Some(11000),
            ..zenbook()
        };
        let snap = inventory(&machine, &[]);
        assert_eq!(snap.source, InventorySource::NvidiaSmi);
        assert_eq!(snap.usable_free_mib, 11000);
        assert_eq!(
            snap.preferred.as_ref().map(|d| d.id.as_str()),
            Some("nvidia")
        );
    }

    #[test]
    fn zero_free_vram_survives_nvidia_and_runner_probes() {
        let (_, total, free) = parse_nvidia_smi("NVIDIA GeForce RTX 4070, 12288 MiB, 0 MiB");
        assert_eq!(free, Some(0));
        let machine = Machine {
            nvidia_driver_usable: true,
            nvidia_memory_mib: total,
            nvidia_free_mib: free,
            ..zenbook()
        };
        let fallback = inventory(&machine, &[]);
        assert_eq!(fallback.total_mib, total);
        assert_eq!(fallback.usable_free_mib, 0);
        assert!(!fallback.has_free(1));

        let devices = parse_device_list("CUDA0: NVIDIA GeForce RTX 4070 (12288 MiB, 0 MiB free)");
        let advice = advise(&machine, "auto", "auto", 2 * 1024 * 1024 * 1024, &devices);
        assert!(!advice.use_graphics);
        assert!(advice.processor_reason.contains("0 MiB free"));
    }

    #[test]
    fn nearly_full_vram_falls_back_to_the_processor_with_a_reason() {
        let devices =
            parse_device_list("  CUDA0: NVIDIA GeForce RTX 4070 (12288 MiB, 128 MiB free)\n");
        let (picked, insufficient) = pick_device(&devices, LFM3_BYTES);
        assert!(picked.is_none());
        assert!(insufficient);
        let advice = advise(&zenbook(), "auto", "auto", LFM3_BYTES, &devices);
        assert!(!advice.use_graphics);
        assert_eq!(advice.args[1], "none");
        assert!(advice.processor_reason.contains("128"));
        assert!(advice.summary.contains("processor"));
        assert!(advice.summary.contains("Not enough free graphics memory"));
        assert_plain(&advice.summary);
    }
}
