use std::{
    collections::HashMap,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    globals::{BindError, GlobalListContents, registry_queue_init},
    protocol::{wl_callback, wl_output, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

const PROFILE_KEYWORD: &[u8] = b"profile";

#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("failed to parse input output JSON")]
    ParseJson(#[source] serde_json::Error),
    #[error("profile name cannot be empty")]
    EmptyProfileName,
    #[error("output `{output}` is enabled but has no current or preferred mode")]
    MissingMode { output: String },
    #[error("output `{output}` is enabled but has no position")]
    MissingPosition { output: String },
    #[error("output `{output}` is enabled but has no scale")]
    MissingScale { output: String },
    #[error("failed to format kanshi profile")]
    Format,
    #[error("could not resolve default kanshi config path: set XDG_CONFIG_HOME or HOME")]
    ConfigPathUnavailable,
    #[error("failed to read kanshi config `{path}`")]
    ConfigRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write kanshi config `{path}`")]
    ConfigWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse kanshi config: {details}")]
    ConfigParse { details: String },
    #[error("found duplicate profile `{profile_name}` in kanshi config ({count} blocks)")]
    DuplicateProfileName { profile_name: String, count: usize },
    #[error("failed to connect to Wayland compositor: {details}")]
    WaylandConnect { details: String },
    #[error(
        "compositor does not support zwlr_output_manager_v1 (wlr-output-management-unstable-v1)"
    )]
    WaylandProtocolUnsupported,
    #[error("failed to communicate with Wayland output-management protocol: {details}")]
    WaylandProtocolError { details: String },
    #[error("timed out waiting for initial output-management state sync")]
    WaylandSyncTimeout,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
struct WlrStatus(Vec<OutputSnapshot>);

#[derive(Debug, Clone, Deserialize)]
pub struct OutputSnapshot {
    name: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    serial: Option<String>,
    enabled: bool,
    #[serde(default)]
    modes: Vec<ModeSnapshot>,
    position: Option<PositionSnapshot>,
    scale: Option<f64>,
    transform: Option<String>,
}

impl OutputSnapshot {
    fn identifier(&self) -> String {
        let mut segments = Vec::with_capacity(3);
        if !self.make.trim().is_empty() {
            segments.push(self.make.as_str());
        }
        if !self.model.trim().is_empty() {
            segments.push(self.model.as_str());
        }
        if let Some(serial) = self.serial.as_deref()
            && !serial.trim().is_empty()
        {
            segments.push(serial);
        }

        if segments.is_empty() {
            self.name.clone()
        } else {
            segments.join(" ")
        }
    }

    fn active_mode(&self) -> Option<&ModeSnapshot> {
        self.modes
            .iter()
            .find(|mode| mode.current)
            .or_else(|| self.modes.iter().find(|mode| mode.preferred))
    }

    fn normalized_transform(&self) -> Option<&'static str> {
        normalize_transform_str(self.transform.as_deref()?)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModeSnapshot {
    width: u32,
    height: u32,
    refresh: f64,
    preferred: bool,
    current: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PositionSnapshot {
    x: i32,
    y: i32,
}

#[derive(Debug)]
struct ProfileBlock {
    name: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Default)]
struct WaylandState {
    done_received: bool,
    finished: bool,
    heads: HashMap<ObjectId, WaylandHeadState>,
    modes: HashMap<ObjectId, WaylandModeState>,
}

#[derive(Debug, Default)]
struct WaylandHeadState {
    name: Option<String>,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    enabled: Option<bool>,
    current_mode: Option<ObjectId>,
    position: Option<PositionSnapshot>,
    scale: Option<f64>,
    transform: Option<String>,
    mode_ids: Vec<ObjectId>,
}

#[derive(Debug, Default)]
struct WaylandModeState {
    width: Option<i32>,
    height: Option<i32>,
    refresh_mhz: Option<i32>,
    preferred: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.heads.entry(head.id()).or_default();
            }
            zwlr_output_manager_v1::Event::Done { .. } => {
                state.done_received = true;
            }
            zwlr_output_manager_v1::Event::Finished => {
                state.finished = true;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(WaylandState, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ())
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let head_state = state.heads.entry(head.id()).or_default();

        match event {
            zwlr_output_head_v1::Event::Name { name } => {
                head_state.name = Some(name);
            }
            zwlr_output_head_v1::Event::Make { make } => {
                head_state.make = Some(make);
            }
            zwlr_output_head_v1::Event::Model { model } => {
                head_state.model = Some(model);
            }
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                head_state.serial = Some(serial_number);
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => {
                head_state.enabled = Some(enabled != 0);
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                let mode_id = mode.id();
                state.modes.entry(mode_id.clone()).or_default();
                if !head_state.mode_ids.contains(&mode_id) {
                    head_state.mode_ids.push(mode_id);
                }
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                head_state.current_mode = Some(mode.id());
            }
            zwlr_output_head_v1::Event::Position { x, y } => {
                head_state.position = Some(PositionSnapshot { x, y });
            }
            zwlr_output_head_v1::Event::Scale { scale } => {
                head_state.scale = Some(scale);
            }
            zwlr_output_head_v1::Event::Transform { transform } => {
                head_state.transform = transform_from_wayland(transform);
            }
            zwlr_output_head_v1::Event::Finished => {
                state.heads.remove(&head.id());
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(WaylandState, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ())
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mode_state = state.modes.entry(mode.id()).or_default();
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                mode_state.width = Some(width);
                mode_state.height = Some(height);
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                mode_state.refresh_mhz = Some(refresh);
            }
            zwlr_output_mode_v1::Event::Preferred => {
                mode_state.preferred = true;
            }
            zwlr_output_mode_v1::Event::Finished => {
                state.modes.remove(&mode.id());
            }
            _ => {}
        }
    }
}

pub fn collect_outputs_wayland() -> Result<Vec<OutputSnapshot>, GenerateError> {
    let connection =
        Connection::connect_to_env().map_err(|source| GenerateError::WaylandConnect {
            details: source.to_string(),
        })?;

    let (globals, mut event_queue) =
        registry_queue_init::<WaylandState>(&connection).map_err(|source| {
            GenerateError::WaylandProtocolError {
                details: source.to_string(),
            }
        })?;
    let qh = event_queue.handle();

    let manager = globals
        .bind::<ZwlrOutputManagerV1, _, _>(&qh, 1..=4, ())
        .map_err(map_bind_error)?;

    let mut state = WaylandState::default();
    let mut synced = false;
    for _ in 0..3 {
        event_queue.roundtrip(&mut state).map_err(|source| {
            GenerateError::WaylandProtocolError {
                details: source.to_string(),
            }
        })?;

        if state.done_received || state.finished {
            synced = true;
            break;
        }
    }

    manager.stop();

    if !synced {
        return Err(GenerateError::WaylandSyncTimeout);
    }

    build_output_snapshots(&state)
}

pub fn collect_outputs_from_json(raw_json: &[u8]) -> Result<Vec<OutputSnapshot>, GenerateError> {
    let status: WlrStatus = serde_json::from_slice(raw_json).map_err(GenerateError::ParseJson)?;
    Ok(status.0)
}

pub fn generate_profile_from_outputs(
    profile_name: &str,
    outputs: &[OutputSnapshot],
) -> Result<String, GenerateError> {
    if profile_name.trim().is_empty() {
        return Err(GenerateError::EmptyProfileName);
    }

    render_profile(profile_name, outputs)
}

pub fn generate_profile_from_slice(
    profile_name: &str,
    raw_json: &[u8],
) -> Result<String, GenerateError> {
    let outputs = collect_outputs_from_json(raw_json)?;
    generate_profile_from_outputs(profile_name, &outputs)
}

pub fn resolve_default_kanshi_config_path() -> Result<PathBuf, GenerateError> {
    if let Some(xdg_config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg_config_home).join("kanshi").join("config"));
    }

    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".config")
            .join("kanshi")
            .join("config"));
    }

    Err(GenerateError::ConfigPathUnavailable)
}

pub fn upsert_profile_in_config(
    config: &str,
    profile_name: &str,
    new_profile_block: &str,
) -> Result<String, GenerateError> {
    if profile_name.trim().is_empty() {
        return Err(GenerateError::EmptyProfileName);
    }

    let blocks = parse_profile_blocks(config)?;
    let mut matches = blocks
        .iter()
        .filter(|block| block.name == profile_name)
        .collect::<Vec<_>>();

    if matches.len() > 1 {
        return Err(GenerateError::DuplicateProfileName {
            profile_name: profile_name.to_owned(),
            count: matches.len(),
        });
    }

    let mut canonical_block = new_profile_block.to_owned();
    if !canonical_block.ends_with('\n') {
        canonical_block.push('\n');
    }

    let mut merged = if matches.is_empty() {
        append_profile(config, &canonical_block)
    } else {
        let target = matches.remove(0);
        let suffix = &config[target.end..];
        let replacement = if suffix.starts_with('\n') && canonical_block.ends_with('\n') {
            canonical_block
                .strip_suffix('\n')
                .unwrap_or(&canonical_block)
        } else {
            &canonical_block
        };
        let mut out = String::with_capacity(config.len() + canonical_block.len());
        out.push_str(&config[..target.start]);
        out.push_str(replacement);
        out.push_str(suffix);
        out
    };

    if !merged.ends_with('\n') {
        merged.push('\n');
    }

    Ok(merged)
}

pub fn upsert_profile_in_file(
    config_path: &Path,
    profile_name: &str,
    new_profile_block: &str,
) -> Result<(), GenerateError> {
    let target_path = if config_path.exists() {
        fs::canonicalize(config_path).unwrap_or_else(|_| config_path.to_path_buf())
    } else {
        config_path.to_path_buf()
    };

    let existing = match fs::read_to_string(&target_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(GenerateError::ConfigRead {
                path: target_path.display().to_string(),
                source,
            });
        }
    };

    let merged = upsert_profile_in_config(&existing, profile_name, new_profile_block)?;
    write_atomic(&target_path, &merged)
}

fn map_bind_error(error: BindError) -> GenerateError {
    match error {
        BindError::NotPresent | BindError::UnsupportedVersion => {
            GenerateError::WaylandProtocolUnsupported
        }
    }
}

fn build_output_snapshots(state: &WaylandState) -> Result<Vec<OutputSnapshot>, GenerateError> {
    let mut outputs = Vec::with_capacity(state.heads.len());

    for head_state in state.heads.values() {
        let output_name = head_state
            .name
            .clone()
            .unwrap_or_else(|| String::from("<unknown>"));

        let mut modes = Vec::new();
        for mode_id in &head_state.mode_ids {
            let Some(mode_state) = state.modes.get(mode_id) else {
                continue;
            };

            let width = mode_state
                .width
                .ok_or_else(|| GenerateError::WaylandProtocolError {
                    details: format!("mode for output `{output_name}` missing width"),
                })
                .and_then(|value| {
                    u32::try_from(value).map_err(|_| GenerateError::WaylandProtocolError {
                        details: format!("mode for output `{output_name}` has negative width"),
                    })
                })?;
            let height = mode_state
                .height
                .ok_or_else(|| GenerateError::WaylandProtocolError {
                    details: format!("mode for output `{output_name}` missing height"),
                })
                .and_then(|value| {
                    u32::try_from(value).map_err(|_| GenerateError::WaylandProtocolError {
                        details: format!("mode for output `{output_name}` has negative height"),
                    })
                })?;
            let refresh =
                mode_state
                    .refresh_mhz
                    .ok_or_else(|| GenerateError::WaylandProtocolError {
                        details: format!("mode for output `{output_name}` missing refresh rate"),
                    })?;

            modes.push(ModeSnapshot {
                width,
                height,
                refresh: f64::from(refresh) / 1000.0,
                preferred: mode_state.preferred,
                current: head_state.current_mode.as_ref() == Some(mode_id),
            });
        }

        let output = OutputSnapshot {
            name: output_name,
            make: head_state.make.clone().unwrap_or_default(),
            model: head_state.model.clone().unwrap_or_default(),
            serial: head_state.serial.clone(),
            enabled: head_state.enabled.unwrap_or(false),
            modes,
            position: head_state.position.clone(),
            scale: head_state.scale,
            transform: head_state.transform.clone(),
        };

        outputs.push(output);
    }

    outputs.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));

    Ok(outputs)
}

fn write_atomic(path: &Path, content: &str) -> Result<(), GenerateError> {
    let parent = path.parent().ok_or_else(|| GenerateError::ConfigWrite {
        path: path.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config path has no parent directory",
        ),
    })?;

    fs::create_dir_all(parent).map_err(|source| GenerateError::ConfigWrite {
        path: parent.display().to_string(),
        source,
    })?;

    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("config");

    let mut temp_path = None;
    let mut temp_file = None;
    for attempt in 0..64_u32 {
        let candidate = parent.join(format!(
            ".{file_name}.kanshi-generate.{}.{}.tmp",
            std::process::id(),
            attempt
        ));

        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(file) => {
                temp_path = Some(candidate);
                temp_file = Some(file);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(GenerateError::ConfigWrite {
                    path: candidate.display().to_string(),
                    source,
                });
            }
        }
    }

    let temp_path = temp_path.ok_or_else(|| GenerateError::ConfigWrite {
        path: path.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "failed to allocate unique temporary file",
        ),
    })?;

    let mut temp_file = temp_file.expect("temp file must exist when temp path exists");

    if let Err(source) = temp_file.write_all(content.as_bytes()) {
        let _ = fs::remove_file(&temp_path);
        return Err(GenerateError::ConfigWrite {
            path: temp_path.display().to_string(),
            source,
        });
    }

    if let Err(source) = temp_file.sync_all() {
        let _ = fs::remove_file(&temp_path);
        return Err(GenerateError::ConfigWrite {
            path: temp_path.display().to_string(),
            source,
        });
    }

    drop(temp_file);

    fs::rename(&temp_path, path).map_err(|source| {
        let _ = fs::remove_file(&temp_path);
        GenerateError::ConfigWrite {
            path: path.display().to_string(),
            source,
        }
    })
}

fn append_profile(config: &str, profile_block: &str) -> String {
    if config.is_empty() {
        return profile_block.to_owned();
    }

    let mut out = String::with_capacity(config.len() + profile_block.len() + 2);
    out.push_str(config);

    if out.ends_with("\n\n") {
        // exactly one blank separator already present
    } else if out.ends_with('\n') {
        out.push('\n');
    } else {
        out.push_str("\n\n");
    }

    out.push_str(profile_block);
    out
}

fn parse_profile_blocks(config: &str) -> Result<Vec<ProfileBlock>, GenerateError> {
    let bytes = config.as_bytes();
    let mut blocks = Vec::new();
    let mut i = 0;
    let mut in_comment = false;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = bytes[i];

        if in_comment {
            if ch == b'\n' {
                in_comment = false;
            }
            i += 1;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match ch {
            b'#' => {
                in_comment = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_string = true;
                i += 1;
                continue;
            }
            b'p' if is_profile_start(bytes, i) => {
                let (block, next_index) = parse_profile_block(config, i)?;
                blocks.push(block);
                i = next_index;
                continue;
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(blocks)
}

fn is_profile_start(bytes: &[u8], index: usize) -> bool {
    let token_end = index + PROFILE_KEYWORD.len();
    if token_end > bytes.len() {
        return false;
    }

    if &bytes[index..token_end] != PROFILE_KEYWORD {
        return false;
    }

    let before_ok = index == 0 || !is_identifier_char(bytes[index - 1]);
    let after_ok = token_end < bytes.len() && bytes[token_end].is_ascii_whitespace();
    before_ok && after_ok
}

fn parse_profile_block(config: &str, start: usize) -> Result<(ProfileBlock, usize), GenerateError> {
    let bytes = config.as_bytes();
    let token_end = start + PROFILE_KEYWORD.len();

    let mut i = token_end;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    let name_start = i;
    let mut in_string = false;
    let mut in_comment = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = bytes[i];

        if in_comment {
            if ch == b'\n' {
                in_comment = false;
            }
            i += 1;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match ch {
            b'#' => {
                in_comment = true;
                i += 1;
            }
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'{' => break,
            _ => i += 1,
        }
    }

    if i >= bytes.len() || bytes[i] != b'{' {
        return Err(GenerateError::ConfigParse {
            details: format!("profile block starting at byte {start} has no opening brace"),
        });
    }

    let name = config[name_start..i].trim().to_owned();
    if name.is_empty() {
        return Err(GenerateError::ConfigParse {
            details: format!("profile block starting at byte {start} has an empty profile name"),
        });
    }

    let mut depth = 1usize;
    let mut j = i + 1;
    in_string = false;
    in_comment = false;
    escaped = false;

    while j < bytes.len() {
        let ch = bytes[j];

        if in_comment {
            if ch == b'\n' {
                in_comment = false;
            }
            j += 1;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            j += 1;
            continue;
        }

        match ch {
            b'#' => {
                in_comment = true;
                j += 1;
            }
            b'"' => {
                in_string = true;
                j += 1;
            }
            b'{' => {
                depth += 1;
                j += 1;
            }
            b'}' => {
                depth -= 1;
                j += 1;
                if depth == 0 {
                    return Ok((
                        ProfileBlock {
                            name,
                            start,
                            end: j,
                        },
                        j,
                    ));
                }
            }
            _ => {
                j += 1;
            }
        }
    }

    Err(GenerateError::ConfigParse {
        details: format!("profile `{name}` has an unclosed block"),
    })
}

fn is_identifier_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-'
}

fn render_profile(profile_name: &str, outputs: &[OutputSnapshot]) -> Result<String, GenerateError> {
    let mut profile = String::with_capacity(32 + outputs.len() * 128);
    writeln!(&mut profile, "profile {profile_name} {{").map_err(|_| GenerateError::Format)?;

    for output in outputs {
        let output_id = escape_kanshi_quoted(&output.identifier());
        if output.enabled {
            let mode = output
                .active_mode()
                .ok_or_else(|| GenerateError::MissingMode {
                    output: output.name.clone(),
                })?;
            let position =
                output
                    .position
                    .as_ref()
                    .ok_or_else(|| GenerateError::MissingPosition {
                        output: output.name.clone(),
                    })?;
            let scale = output.scale.ok_or_else(|| GenerateError::MissingScale {
                output: output.name.clone(),
            })?;
            if let Some(transform) = output.normalized_transform() {
                writeln!(
                    &mut profile,
                    "  output \"{output_id}\" mode {}x{}@{:.2}Hz position {},{} scale {:.2} transform {transform}",
                    mode.width, mode.height, mode.refresh, position.x, position.y, scale
                )
                .map_err(|_| GenerateError::Format)?;
            } else {
                writeln!(
                    &mut profile,
                    "  output \"{output_id}\" mode {}x{}@{:.2}Hz position {},{} scale {:.2}",
                    mode.width, mode.height, mode.refresh, position.x, position.y, scale
                )
                .map_err(|_| GenerateError::Format)?;
            }
        } else {
            writeln!(&mut profile, "  output \"{output_id}\" disable")
                .map_err(|_| GenerateError::Format)?;
        }
    }

    profile.push_str("}\n");
    Ok(profile)
}

fn escape_kanshi_quoted(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn transform_from_wayland(
    transform: wayland_client::WEnum<wl_output::Transform>,
) -> Option<String> {
    match transform {
        wayland_client::WEnum::Value(wl_output::Transform::Normal) => Some(String::from("normal")),
        wayland_client::WEnum::Value(wl_output::Transform::_90) => Some(String::from("90")),
        wayland_client::WEnum::Value(wl_output::Transform::_180) => Some(String::from("180")),
        wayland_client::WEnum::Value(wl_output::Transform::_270) => Some(String::from("270")),
        wayland_client::WEnum::Value(wl_output::Transform::Flipped) => {
            Some(String::from("flipped"))
        }
        wayland_client::WEnum::Value(wl_output::Transform::Flipped90) => {
            Some(String::from("flipped-90"))
        }
        wayland_client::WEnum::Value(wl_output::Transform::Flipped180) => {
            Some(String::from("flipped-180"))
        }
        wayland_client::WEnum::Value(wl_output::Transform::Flipped270) => {
            Some(String::from("flipped-270"))
        }
        wayland_client::WEnum::Value(_) => None,
        wayland_client::WEnum::Unknown(raw) => normalize_transform_u32(raw).map(String::from),
    }
}

fn normalize_transform_u32(raw: u32) -> Option<&'static str> {
    match raw {
        0 => Some("normal"),
        1 => Some("90"),
        2 => Some("180"),
        3 => Some("270"),
        4 => Some("flipped"),
        5 => Some("flipped-90"),
        6 => Some("flipped-180"),
        7 => Some("flipped-270"),
        _ => None,
    }
}

fn normalize_transform_str(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "normal" => Some("normal"),
        "90" => Some("90"),
        "180" => Some("180"),
        "270" => Some("270"),
        "flipped" => Some("flipped"),
        "flipped-90" => Some("flipped-90"),
        "flipped-180" => Some("flipped-180"),
        "flipped-270" => Some("flipped-270"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{Mutex, OnceLock},
    };

    use super::{
        GenerateError, collect_outputs_from_json, generate_profile_from_slice,
        resolve_default_kanshi_config_path, upsert_profile_in_config,
    };

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_locked_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().expect("env mutex poisoned");
        let old_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let old_home = std::env::var_os("HOME");
        let result = f();
        unsafe {
            match old_xdg {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        result
    }

    #[test]
    fn renders_fixture_with_enabled_and_disabled_outputs() {
        let json = include_str!("../tests/fixtures/mixed_outputs.json");
        let expected = include_str!("../tests/fixtures/mixed_outputs.kanshi");
        let rendered = generate_profile_from_slice("docked", json.as_bytes()).unwrap();
        assert_eq!(rendered, expected);
    }

    #[test]
    fn json_collection_parses_fixture() {
        let json = include_str!("../tests/fixtures/mixed_outputs.json");
        let outputs = collect_outputs_from_json(json.as_bytes()).unwrap();
        assert_eq!(outputs.len(), 3);
    }

    #[test]
    fn picks_current_mode_first() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":false},
              {"width":2560,"height":1440,"refresh":59.95,"preferred":false,"current":true}
            ],
            "position":{"x":10,"y":20},
            "scale":1.0
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("mode 2560x1440@59.95Hz"));
    }

    #[test]
    fn falls_back_to_preferred_mode() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":false}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("mode 1920x1080@60.00Hz"));
    }

    #[test]
    fn errors_when_enabled_output_has_no_current_or_preferred_mode() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":false,"current":false}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0
          }
        ]"#;
        let err = generate_profile_from_slice("desk", json.as_bytes()).unwrap_err();
        assert!(matches!(err, GenerateError::MissingMode { .. }));
    }

    #[test]
    fn keeps_negative_coordinates() {
        let json = r#"[
          {
            "name":"DP-2",
            "make":"Dell",
            "model":"P2723D",
            "serial":"2ZZ6714",
            "enabled":true,
            "modes":[
              {"width":2560,"height":1440,"refresh":59.951,"preferred":true,"current":true}
            ],
            "position":{"x":-2560,"y":300},
            "scale":1.25
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("position -2560,300"));
    }

    #[test]
    fn omits_unknown_serial_placeholder() {
        let json = r#"[
          {
            "name":"eDP-1",
            "make":"AU Optronics",
            "model":"0xD291",
            "serial":null,
            "enabled":false,
            "modes":[
              {"width":1920,"height":1200,"refresh":60.0,"preferred":true,"current":false}
            ]
          }
        ]"#;
        let rendered = generate_profile_from_slice("mobile", json.as_bytes()).unwrap();
        assert!(rendered.contains("output \"AU Optronics 0xD291\" disable"));
        assert!(!rendered.contains("Unknown"));
    }

    #[test]
    fn includes_transform_for_rotated_enabled_output() {
        let json = r#"[
          {
            "name":"DP-7",
            "make":"Dell Inc.",
            "model":"DELL U2422H",
            "serial":"75BNF83",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":true}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0,
            "transform":"90"
          }
        ]"#;
        let rendered = generate_profile_from_slice("rotated", json.as_bytes()).unwrap();
        assert!(rendered.contains("transform 90"));
    }

    #[test]
    fn includes_transform_for_normal_orientation() {
        let json = r#"[
          {
            "name":"DP-1",
            "make":"Dell",
            "model":"U2723",
            "serial":"ABC123",
            "enabled":true,
            "modes":[
              {"width":1920,"height":1080,"refresh":60.0,"preferred":true,"current":true}
            ],
            "position":{"x":0,"y":0},
            "scale":1.0,
            "transform":"normal"
          }
        ]"#;
        let rendered = generate_profile_from_slice("desk", json.as_bytes()).unwrap();
        assert!(rendered.contains("transform normal"));
    }

    #[test]
    fn resolve_default_config_uses_xdg_config_home() {
        with_locked_env(|| {
            let temp = tempfile::TempDir::new().unwrap();
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", temp.path());
                std::env::remove_var("HOME");
            }

            let path = resolve_default_kanshi_config_path().unwrap();
            assert_eq!(path, temp.path().join("kanshi").join("config"));
        });
    }

    #[test]
    fn resolve_default_config_falls_back_to_home() {
        with_locked_env(|| {
            let temp = tempfile::TempDir::new().unwrap();
            unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
                std::env::set_var("HOME", temp.path());
            }

            let path = resolve_default_kanshi_config_path().unwrap();
            assert_eq!(
                path,
                temp.path().join(".config").join("kanshi").join("config")
            );
        });
    }

    #[test]
    fn upsert_replaces_single_matching_block() {
        let current = "# header\nprofile desk {\n  output \"old\" disable\n}\n\nprofile other {\n  output \"x\" disable\n}\n";
        let replacement = "profile desk {\n  output \"new\" disable\n}\n";

        let merged = upsert_profile_in_config(current, "desk", replacement).unwrap();

        assert!(merged.contains("output \"new\" disable"));
        assert!(merged.contains("profile other"));
        assert!(!merged.contains("output \"old\" disable"));
    }

    #[test]
    fn upsert_appends_when_profile_is_missing() {
        let current = "profile alpha {\n  output \"x\" disable\n}";
        let inserted = "profile beta {\n  output \"y\" disable\n}\n";

        let merged = upsert_profile_in_config(current, "beta", inserted).unwrap();

        assert!(merged.starts_with("profile alpha"));
        assert!(merged.contains("\n\nprofile beta"));
        assert!(merged.ends_with('\n'));
    }

    #[test]
    fn upsert_fails_on_duplicate_matching_profile_names() {
        let current = "profile desk {\n}\nprofile desk {\n}\n";
        let inserted = "profile desk {\n  output \"x\" disable\n}\n";

        let err = upsert_profile_in_config(current, "desk", inserted).unwrap_err();
        assert!(matches!(err, GenerateError::DuplicateProfileName { .. }));
    }

    #[test]
    fn upsert_does_not_match_partial_profile_names() {
        let current = "profile Home-21-9 {\n}\n";
        let inserted = "profile Home {\n}\n";

        let merged = upsert_profile_in_config(current, "Home", inserted).unwrap();
        assert!(merged.contains("profile Home-21-9"));
        assert!(merged.contains("profile Home {"));
    }

    #[test]
    fn parser_ignores_profile_keyword_in_comments_and_strings() {
        let current =
            "# profile fake { }\nprofile desk {\n  output \"literal profile ignored\" disable\n}\n";
        let inserted = "profile desk {\n  output \"new\" disable\n}\n";

        let merged = upsert_profile_in_config(current, "desk", inserted).unwrap();
        assert!(merged.starts_with("# profile fake { }\n"));
        assert!(merged.contains("output \"new\" disable"));
    }

    #[test]
    fn parser_reports_unclosed_profile_block() {
        let current = "profile broken {\n  output \"x\" disable\n";
        let inserted = "profile broken {\n}\n";

        let err = upsert_profile_in_config(current, "broken", inserted).unwrap_err();
        assert!(matches!(err, GenerateError::ConfigParse { .. }));
    }

    #[test]
    fn appended_content_ensures_single_newline_termination() {
        let current = "profile alpha {\n}\n";
        let inserted = "profile beta {\n}";
        let merged = upsert_profile_in_config(current, "beta", inserted).unwrap();
        assert!(merged.ends_with('\n'));
    }

    #[test]
    fn resolve_default_config_requires_home_or_xdg() {
        with_locked_env(|| {
            unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
                std::env::remove_var("HOME");
            }
            let err = resolve_default_kanshi_config_path().unwrap_err();
            assert!(matches!(err, GenerateError::ConfigPathUnavailable));

            let fallback = Path::new("/tmp");
            unsafe {
                std::env::set_var("HOME", fallback);
            }
        });
    }
}
