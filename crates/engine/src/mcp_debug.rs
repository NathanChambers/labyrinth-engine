use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread;
use std::time::Duration;

use math::Camera;
use renderer::{CapturedDepthBuffer, CapturedFrame, RenderDebugMode, Renderer};

static CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct McpDebugRequest {
    operation: McpDebugOperation,
    reply: SyncSender<Result<McpDebugResponse, String>>,
}

enum McpDebugOperation {
    Capture { buffer: CaptureBuffer, width: u32, height: u32 },
    Pick { x: u32, y: u32, width: u32, height: u32 },
    GridInfo,
    Probe { x: u32, y: u32, z: u32 },
    SdfVoxel { x: u32, y: u32, z: u32 },
    Pause,
    Resume,
    Status,
    PerformanceStats,
    Profile { frame_count: u32, include_gpu_timing: bool },
}

enum McpDebugResponse {
    Image(PathBuf, u64, bool),
    Json(String, u64, bool),
}

#[derive(Default)]
pub(crate) struct McpProfiler {
    pending: Option<PendingProfile>,
}

struct PendingProfile {
    reply: SyncSender<Result<McpDebugResponse, String>>,
    requested_frames: u32,
    include_gpu_timing: bool,
    gpu_timing_available: bool,
    start_frame_id: u64,
    samples: Vec<PerformanceSample>,
}

struct PerformanceSample {
    frame_id: u64,
    stats: renderer::RendererPerformanceStats,
}

#[derive(Clone, Copy, Debug)]
enum CaptureBuffer {
    Color,
    SceneDepth,
    SurfaceNormals,
    DirectionalShadow(usize),
    SpotShadow,
    PointShadow(usize),
    Unlit,
    Wireframe,
    ShadowVisibility,
    GiOnly,
    GiSupport,
}

impl CaptureBuffer {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "color" => Some(Self::Color),
            "depth" => Some(Self::SceneDepth),
            "normals" => Some(Self::SurfaceNormals),
            "directional_shadow_0" => Some(Self::DirectionalShadow(0)),
            "directional_shadow_1" => Some(Self::DirectionalShadow(1)),
            "directional_shadow_2" => Some(Self::DirectionalShadow(2)),
            "spot_shadow" => Some(Self::SpotShadow),
            "point_shadow_0" => Some(Self::PointShadow(0)),
            "point_shadow_1" => Some(Self::PointShadow(1)),
            "point_shadow_2" => Some(Self::PointShadow(2)),
            "point_shadow_3" => Some(Self::PointShadow(3)),
            "point_shadow_4" => Some(Self::PointShadow(4)),
            "point_shadow_5" => Some(Self::PointShadow(5)),
            "point_shadow_6" => Some(Self::PointShadow(6)),
            "point_shadow_7" => Some(Self::PointShadow(7)),
            "point_shadow_8" => Some(Self::PointShadow(8)),
            "point_shadow_9" => Some(Self::PointShadow(9)),
            "point_shadow_10" => Some(Self::PointShadow(10)),
            "point_shadow_11" => Some(Self::PointShadow(11)),
            "unlit" => Some(Self::Unlit),
            "wireframe" => Some(Self::Wireframe),
            "shadow_visibility" => Some(Self::ShadowVisibility),
            "gi_only" => Some(Self::GiOnly),
            "gi_support" => Some(Self::GiSupport),
            _ => None,
        }
    }

    fn debug_mode(self) -> Option<RenderDebugMode> {
        match self {
            Self::Unlit => Some(RenderDebugMode::UnlitMaterials),
            Self::Wireframe => Some(RenderDebugMode::Wireframe),
            Self::ShadowVisibility => Some(RenderDebugMode::ShadowVisibility),
            Self::GiOnly => Some(RenderDebugMode::GiOnly),
            Self::GiSupport => Some(RenderDebugMode::GiSupport),
            _ => None,
        }
    }
}

pub(crate) fn start_from_env() -> Result<Option<Receiver<McpDebugRequest>>, String> {
    let Ok(port) = std::env::var("LABYRINTH_MCP_PORT") else {
        return Ok(None);
    };
    let port = port.parse::<u16>().map_err(|error| format!("invalid LABYRINTH_MCP_PORT: {error}"))?;
    if port == 0 {
        return Err("LABYRINTH_MCP_PORT must be between 1 and 65535".into());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| format!("failed to start local MCP debug endpoint on port {port}: {error}"))?;
    let (sender, requests) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sender = sender.clone();
            thread::spawn(move || handle_connection(stream, sender));
        }
    });
    crate::info!(port; "MCP debug endpoint listening on loopback");
    Ok(Some(requests))
}

fn handle_connection(mut stream: TcpStream, sender: Sender<McpDebugRequest>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream).take(512);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let fields = request.split_ascii_whitespace().collect::<Vec<_>>();
    let parsed = match fields.as_slice() {
        ["capture", buffer, width, height] => {
            let capture_buffer = CaptureBuffer::parse(buffer);
            let dimensions = width.parse::<u32>().ok().zip(height.parse::<u32>().ok());
            match (capture_buffer, dimensions) {
                (Some(buffer), Some((0, 0))) => Ok(McpDebugOperation::Capture { buffer, width: 0, height: 0 }),
                (Some(buffer), Some((width, height))) if width > 0 && height > 0 && width <= 4096 && height <= 4096 => Ok(McpDebugOperation::Capture { buffer, width, height }),
                (None, _) => Err(format!("unsupported render buffer '{buffer}'")),
                (_, None) => Err("capture dimensions must be integers".into()),
                _ => Err("capture dimensions must be between 1 and 4096".into()),
            }
        }
        ["pick", x, y, width, height] => match (x.parse::<u32>(), y.parse::<u32>(), width.parse::<u32>(), height.parse::<u32>()) {
            (Ok(x), Ok(y), Ok(0), Ok(0)) if x <= 4096 && y <= 4096 => Ok(McpDebugOperation::Pick { x, y, width: 0, height: 0 }),
            (Ok(x), Ok(y), Ok(width), Ok(height)) if width > 0 && height > 0 && width <= 4096 && height <= 4096 && x < width && y < height => Ok(McpDebugOperation::Pick { x, y, width, height }),
            _ => Err("pick coordinates must be inside a viewport from 1x1 to 4096x4096".into()),
        },
        ["grid_info"] => Ok(McpDebugOperation::GridInfo),
        ["pause"] => Ok(McpDebugOperation::Pause),
        ["resume"] => Ok(McpDebugOperation::Resume),
        ["status"] => Ok(McpDebugOperation::Status),
        ["performance_stats"] => Ok(McpDebugOperation::PerformanceStats),
        ["profile", frame_count, include_gpu_timing] => match (frame_count.parse::<u32>(), *include_gpu_timing) {
            (Ok(frame_count), "true") if (1..=120).contains(&frame_count) => Ok(McpDebugOperation::Profile { frame_count, include_gpu_timing: true }),
            (Ok(frame_count), "false") if (1..=120).contains(&frame_count) => Ok(McpDebugOperation::Profile { frame_count, include_gpu_timing: false }),
            (Ok(_), _) => Err("profile frame count must be from 1 to 120 and GPU timing must be true or false".into()),
            _ => Err("profile frame count must be an integer".into()),
        },
        ["probe", x, y, z] => match (x.parse::<u32>(), y.parse::<u32>(), z.parse::<u32>()) {
            (Ok(x), Ok(y), Ok(z)) => Ok(McpDebugOperation::Probe { x, y, z }),
            _ => Err("probe coordinates must be non-negative integers".into()),
        },
        ["sdf_voxel", x, y, z] => match (x.parse::<u32>(), y.parse::<u32>(), z.parse::<u32>()) {
            (Ok(x), Ok(y), Ok(z)) => Ok(McpDebugOperation::SdfVoxel { x, y, z }),
            _ => Err("SDF voxel coordinates must be non-negative integers".into()),
        },
        _ => Err("expected: capture <buffer> <width> <height>, pick <x> <y> <width> <height>, grid_info, probe <x> <y> <z>, sdf_voxel <x> <y> <z>, performance_stats, profile <frames> <true|false>, pause, resume, or status".into()),
    };
    let result = parsed.and_then(|operation| {
        let (reply, response) = mpsc::sync_channel(1);
        let profile_timeout = matches!(&operation, McpDebugOperation::Profile { .. });
        sender.send(McpDebugRequest { operation, reply }).map_err(|_| "engine debug endpoint is stopping".to_owned())?;
        let timeout = if profile_timeout { Duration::from_secs(180) } else { Duration::from_secs(30) };
        response.recv_timeout(timeout).map_err(|_| "timed out waiting for the engine debug endpoint".to_owned())?
    });
    match result {
        Ok(McpDebugResponse::Image(path, frame_id, paused)) => match fs::read(path) {
            Ok(image) => write_payload(&mut stream, &image, frame_id, paused),
            Err(error) => write_error(&mut stream, &format!("failed to read rendered image: {error}")),
        },
        Ok(McpDebugResponse::Json(json, frame_id, paused)) => write_payload(&mut stream, json.as_bytes(), frame_id, paused),
        Err(error) => {
            write_error(&mut stream, &error);
        }
    }
}

fn write_payload(stream: &mut TcpStream, payload: &[u8], frame_id: u64, paused: bool) {
    let _ = writeln!(stream, "OK\t{}\t{}\t{}", payload.len(), frame_id, paused);
    let _ = stream.write_all(payload);
}

fn write_error(stream: &mut TcpStream, error: &str) {
    let _ = writeln!(stream, "ERR\t{}", error.replace(['\r', '\n', '\t'], " "));
}

pub(crate) fn process_requests(requests: &Receiver<McpDebugRequest>, renderer: &mut Renderer, camera: &Camera, output_root: &Path, paused: &mut bool, frame_id: u64, profiler: &mut McpProfiler) {
    loop {
        let request = match requests.try_recv() {
            Ok(request) => request,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        };
        let McpDebugRequest { operation, reply } = request;
        if let McpDebugOperation::Profile { frame_count, include_gpu_timing } = &operation {
            if *paused {
                let _ = reply.send(Err("resume the engine before profiling consecutive frames".into()));
            } else if profiler.pending.is_some() {
                let _ = reply.send(Err("a frame profile is already running".into()));
            } else {
                profiler.pending = Some(PendingProfile {
                    reply,
                    requested_frames: *frame_count,
                    include_gpu_timing: *include_gpu_timing,
                    gpu_timing_available: renderer.gpu_timing_available(),
                    start_frame_id: frame_id,
                    samples: Vec::with_capacity(*frame_count as usize),
                });
            }
            continue;
        }
        let result = match operation {
            McpDebugOperation::Capture { buffer, width, height } => capture_to_file(renderer, camera, output_root, buffer, width, height).map(|path| McpDebugResponse::Image(path, frame_id, *paused)),
            McpDebugOperation::Pick { x, y, width, height } => renderer
                .pick_viewport_pixel(camera, x, y, width, height)
                .map(|pick| McpDebugResponse::Json(viewport_pick_json(pick), 0, false))
                .ok_or_else(|| "no rendered surface at that viewport pixel".to_owned()),
            McpDebugOperation::GridInfo => Ok(McpDebugResponse::Json(grid_info_json(renderer), 0, false)),
            McpDebugOperation::Probe { x, y, z } => renderer
                .inspect_radiance_probe([x, y, z])
                .and_then(|probe| probe.map(|probe| McpDebugResponse::Json(radiance_probe_json(&probe), 0, false)).ok_or_else(|| "radiance cell coordinate is outside the field".to_owned())),
            McpDebugOperation::SdfVoxel { x, y, z } => renderer
                .inspect_sdf_voxel([x, y, z])
                .and_then(|voxel| voxel.map(|voxel| McpDebugResponse::Json(sdf_voxel_json(voxel), 0, false)).ok_or_else(|| "SDF voxel coordinate is outside the grid".to_owned())),
            McpDebugOperation::Pause => {
                *paused = true;
                Ok(McpDebugResponse::Json("{\"state\":\"paused\"}".into(), 0, false))
            }
            McpDebugOperation::Resume => {
                *paused = false;
                Ok(McpDebugResponse::Json("{\"state\":\"running\"}".into(), 0, false))
            }
            McpDebugOperation::Status => Ok(McpDebugResponse::Json("{}".into(), 0, false)),
            McpDebugOperation::PerformanceStats => performance_stats_response(renderer),
            McpDebugOperation::Profile { .. } => unreachable!(),
        };
        let result = result.map(|response| match response {
            McpDebugResponse::Image(path, frame_id, paused) => McpDebugResponse::Image(path, frame_id, paused),
            McpDebugResponse::Json(json, _, _) => McpDebugResponse::Json(frame_envelope_json(frame_id, *paused, &json), frame_id, *paused),
        });
        let _ = reply.send(result);
    }

    collect_profile_sample(profiler, renderer, frame_id, *paused);
}

fn performance_stats_response(renderer: &mut Renderer) -> Result<McpDebugResponse, String> {
    renderer.update_gpu_timestamps()?;
    renderer.read_triangle_bvh_diagnostics()?;
    let stats = renderer.performance_stats();
    let gpu_timing_available = renderer.gpu_timing_available();
    let data = performance_stats_json(stats, gpu_timing_available, gpu_timing_available);
    Ok(McpDebugResponse::Json(data, 0, false))
}

fn collect_profile_sample(profiler: &mut McpProfiler, renderer: &mut Renderer, frame_id: u64, paused: bool) {
    let Some(pending) = profiler.pending.as_mut() else {
        return;
    };
    if paused {
        let pending = profiler.pending.take().unwrap();
        finish_profile(pending, frame_id, false, true, Some("paused"));
        return;
    }
    if frame_id <= pending.start_frame_id {
        return;
    }
    if pending.include_gpu_timing && pending.gpu_timing_available {
        if let Err(error) = renderer.update_gpu_timestamps() {
            let pending = profiler.pending.take().unwrap();
            let _ = pending.reply.send(Err(format!("GPU timestamp readback failed: {error}")));
            return;
        }
    }
    pending.samples.push(PerformanceSample { frame_id, stats: renderer.performance_stats() });
    if pending.samples.len() >= pending.requested_frames as usize {
        let pending = profiler.pending.take().unwrap();
        finish_profile(pending, frame_id, true, false, None);
    }
}

fn finish_profile(pending: PendingProfile, frame_id: u64, complete: bool, paused: bool, stop_reason: Option<&str>) {
    let gpu_timing_sampled = pending.include_gpu_timing && pending.gpu_timing_available;
    let samples = pending
        .samples
        .iter()
        .map(|sample| format!("{{\"frame_id\":{},\"stats\":{}}}", sample.frame_id, performance_stats_json(sample.stats, gpu_timing_sampled, pending.gpu_timing_available)))
        .collect::<Vec<_>>()
        .join(",");
    let frame_cpu = pending.samples.iter().map(|sample| sample.stats.frame_cpu_ms).collect::<Vec<_>>();
    let gpu_total = pending.samples.iter().map(|sample| sample.stats.gpu_total_ms).collect::<Vec<_>>();
    let gpu_summary = if gpu_timing_sampled { metric_summary_json(&gpu_total) } else { "null".into() };
    let reason = stop_reason.map(json_string).unwrap_or_else(|| "null".into());
    let data = format!(
        "{{\"requested_frames\":{},\"sample_count\":{},\"complete\":{},\"stop_reason\":{},\"gpu_timing_requested\":{},\"gpu_timing_available\":{},\"summary\":{{\"frame_cpu_ms\":{},\"gpu_total_ms\":{}}},\"samples\":[{}]}}",
        pending.requested_frames,
        pending.samples.len(),
        complete,
        reason,
        pending.include_gpu_timing,
        pending.gpu_timing_available,
        metric_summary_json(&frame_cpu),
        gpu_summary,
        samples
    );
    let result = Ok(McpDebugResponse::Json(data, frame_id, paused));
    let _ = pending.reply.send(result);
}

fn performance_stats_json(stats: renderer::RendererPerformanceStats, include_gpu_timing: bool, gpu_timing_available: bool) -> String {
    let gpu = if include_gpu_timing {
        format!(
            "{{\"shadow_ms\":{},\"scene_ms\":{},\"sdf_gi_ms\":{},\"sdf_update_ms\":{},\"gizmo_ms\":{},\"ui_ms\":{},\"total_ms\":{}}}",
            stats.gpu_shadow_ms, stats.gpu_scene_ms, stats.gpu_sdf_gi_ms, stats.gpu_sdf_update_ms, stats.gpu_gizmo_ms, stats.gpu_ui_ms, stats.gpu_total_ms
        )
    } else {
        "null".into()
    };
    format!(
        "{{\"cpu_ms\":{{\"frame\":{},\"light_update\":{},\"instance_update\":{},\"sdf_build\":{},\"sdf_geometry_bvh_build\":{},\"sdf_voxel_build\":{},\"sdf_clearance_build\":{},\"sdf_upload\":{},\"probe_update\":{},\"irradiance_upload\":{},\"visibility_rebuild\":{},\"triangle_bvh_build\":{},\"triangle_bvh_refit\":{}}},\"gpu_ms\":{},\"gpu_timing_available\":{},\"gpu_timing_sampled\":{},\"work\":{{\"sdf_voxels_updated\":{},\"gi_pending_cells\":{},\"gi_cells_updated\":{},\"gi_rays_scheduled\":{},\"gi_angular_regions\":{},\"probes_updated\":{},\"probe_budget\":{},\"static_sample_count\":{},\"dynamic_sample_count\":{},\"region_count\":{},\"scheduled_shadow_lights\":{},\"dirty_shadow_lights\":{},\"triangle_bvh_segments\":{},\"triangle_bvh_bounds_tests\":{},\"triangle_bvh_triangle_tests\":{},\"triangle_bvh_hits\":{},\"triangle_bvh_node_count\":{},\"triangle_bvh_triangle_count\":{} }},\"gi\":{{\"coverage_available\":{},\"region_coverage_percent\":{},\"regions_covered_twice_percent\":{},\"cells_with_coverage_percent\":{},\"valid_cells_percent\":{},\"regions_attempted_percent\":{}}}}}",
        stats.frame_cpu_ms,
        stats.light_update_ms,
        stats.instance_update_ms,
        stats.sdf_build_cpu_ms,
        stats.sdf_geometry_bvh_build_ms,
        stats.sdf_voxel_build_ms,
        stats.sdf_clearance_build_ms,
        stats.sdf_upload_ms,
        stats.probe_update_ms,
        stats.irradiance_upload_ms,
        stats.visibility_rebuild_ms,
        stats.triangle_bvh_build_ms,
        stats.triangle_bvh_refit_ms,
        gpu,
        gpu_timing_available,
        include_gpu_timing,
        stats.sdf_voxels_updated,
        stats.gi_pending_cells,
        stats.gi_cells_updated,
        stats.gi_rays_scheduled,
        stats.gi_angular_regions,
        stats.probes_updated,
        stats.probe_budget,
        stats.static_sample_count,
        stats.dynamic_sample_count,
        stats.region_count,
        stats.scheduled_shadow_lights,
        stats.dirty_shadow_lights,
        stats.triangle_bvh_segments,
        stats.triangle_bvh_bounds_tests,
        stats.triangle_bvh_triangle_tests,
        stats.triangle_bvh_hits,
        stats.triangle_bvh_node_count,
        stats.triangle_bvh_triangle_count,
        stats.gi_coverage_available,
        stats.gi_region_coverage_percent,
        stats.gi_regions_covered_twice_percent,
        stats.gi_cells_with_coverage_percent,
        stats.gi_valid_cells_percent,
        stats.gi_regions_attempted_percent
    )
}

fn metric_summary_json(values: &[f32]) -> String {
    if values.is_empty() {
        return "null".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let percentile = |fraction: f32| sorted[((sorted.len() as f32 * fraction).ceil() as usize).saturating_sub(1).min(sorted.len() - 1)];
    let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
    format!("{{\"mean\":{},\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}}", mean, percentile(0.50), percentile(0.95), percentile(0.99), sorted[sorted.len() - 1])
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn frame_envelope_json(frame_id: u64, paused: bool, data: &str) -> String {
    format!("{{\"frame_id\":{frame_id},\"paused\":{paused},\"data\":{data}}}")
}

fn grid_info_json(renderer: &Renderer) -> String {
    let sdf = renderer.sdf_grid();
    let radiance = renderer.radiance_field_grid();
    let stats = renderer.performance_stats();
    format!(
        "{{\"sdf\":{{\"minimum\":{},\"maximum\":{},\"resolution\":{},\"voxel_size\":{}}},\"radiance\":{{\"minimum\":{},\"maximum\":{},\"resolution\":{},\"cell_size\":{}}},\"updates\":{{\"cells_updated\":{},\"rays_scheduled\":{},\"pending_cells\":{},\"valid_cells_percent\":{},\"regions_covered_percent\":{}}}}}",
        json_vec3(sdf.minimum),
        json_vec3(sdf.maximum),
        json_u32x3(sdf.resolution),
        json_vec3(sdf.voxel_size),
        json_vec3(radiance.minimum),
        json_vec3(radiance.maximum),
        json_u32x3(radiance.resolution),
        json_cell_size(radiance.minimum, radiance.maximum, radiance.resolution),
        stats.gi_cells_updated,
        stats.gi_rays_scheduled,
        stats.gi_pending_cells,
        stats.gi_valid_cells_percent,
        stats.gi_region_coverage_percent
    )
}

fn viewport_pick_json(pick: renderer::RendererViewportPick) -> String {
    format!(
        "{{\"world_position\":{},\"world_normal\":{},\"distance\":{},\"instance_index\":{},\"triangle_index\":{},\"barycentric\":{},\"sdf_voxel\":{},\"radiance_cell\":{}}}",
        json_vec3(pick.world_position),
        json_vec3(pick.world_normal),
        pick.distance,
        pick.instance_index,
        pick.triangle_index,
        json_vec3(pick.barycentric),
        json_option_u32x3(pick.sdf_voxel),
        json_option_u32x3(pick.radiance_cell)
    )
}

fn json_vec3(value: [f32; 3]) -> String {
    format!("[{},{},{}]", value[0], value[1], value[2])
}

fn json_u32x3(value: [u32; 3]) -> String {
    format!("[{},{},{}]", value[0], value[1], value[2])
}

fn json_cell_size(minimum: [f32; 3], maximum: [f32; 3], resolution: [u32; 3]) -> String {
    json_vec3(std::array::from_fn(|axis| (maximum[axis] - minimum[axis]) / resolution[axis] as f32))
}

fn json_option_u32x3(value: Option<[u32; 3]>) -> String {
    value.map(json_u32x3).unwrap_or_else(|| "null".into())
}

fn radiance_probe_json(probe: &renderer::RendererRadianceProbeDebug) -> String {
    let segments = probe
        .angular_segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "{{\"index\":{},\"sampled\":{},\"direction\":{},\"radiance\":{},\"sample_count\":{},\"attempt_count\":{},\"age_frames\":{}}}",
                index,
                segment.sampled,
                json_vec3(segment.direction),
                json_vec3(segment.radiance),
                segment.sample_count,
                segment.attempt_count,
                segment.age_frames
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"coordinate\":{},\"cell_center\":{},\"probe_position\":{},\"relocation_valid\":{},\"relocation_succeeded\":{},\"covered_once\":{},\"covered_twice\":{},\"attempted_regions\":{},\"angular_segments\":[{}]}}",
        json_u32x3(probe.coordinate),
        json_vec3(probe.cell_center),
        json_vec3(probe.probe_position),
        probe.relocation_valid,
        probe.relocation_succeeded,
        probe.covered_once,
        probe.covered_twice,
        probe.attempted_regions,
        segments
    )
}

fn sdf_voxel_json(voxel: renderer::RendererSdfVoxelDebug) -> String {
    format!(
        "{{\"coordinate\":{},\"world_position\":{},\"signed_distance\":{},\"albedo\":{},\"emission\":{},\"clearance\":{}}}",
        json_u32x3(voxel.coordinate),
        json_vec3(voxel.world_position),
        voxel.signed_distance,
        json_vec3(voxel.albedo),
        json_vec3(voxel.emission),
        voxel.clearance
    )
}

fn capture_to_file(renderer: &mut Renderer, camera: &Camera, output_root: &Path, buffer: CaptureBuffer, width: u32, height: u32) -> Result<PathBuf, String> {
    let frame = match (buffer.debug_mode(), width == 0 && height == 0) {
        (Some(debug_mode), true) => renderer.capture_current_read_only_with_debug_mode(camera, debug_mode)?,
        (Some(debug_mode), false) => renderer.capture_read_only_with_debug_mode(camera, width, height, debug_mode)?,
        (None, true) => renderer.capture_current_read_only(camera)?,
        (None, false) => renderer.capture_read_only(camera, width, height)?,
    };
    let directory = output_root.join("mcp");
    fs::create_dir_all(&directory).map_err(|error| format!("failed to create MCP capture directory: {error}"))?;
    let capture_id = CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path = directory.join(format!("frame_{capture_id:06}.png"));
    write_buffer_png(&path, &frame, buffer, camera.near_clip, camera.far_clip)?;
    Ok(path)
}

fn write_buffer_png(path: &Path, frame: &CapturedFrame, buffer: CaptureBuffer, near_clip: f32, far_clip: f32) -> Result<(), String> {
    match buffer {
        CaptureBuffer::Color | CaptureBuffer::Unlit | CaptureBuffer::Wireframe | CaptureBuffer::ShadowVisibility | CaptureBuffer::GiOnly | CaptureBuffer::GiSupport => {
            write_rgba_png(path, frame.width, frame.height, &frame.rgba8)
        }
        CaptureBuffer::SurfaceNormals => {
            let pixels = frame.surface_normals.iter().flatten().copied().collect::<Vec<_>>();
            write_rgba_png(path, frame.width, frame.height, &pixels)
        }
        CaptureBuffer::SceneDepth => write_depth_png(path, &frame.scene_depth, Some((near_clip, far_clip)), 0),
        CaptureBuffer::DirectionalShadow(layer) => write_depth_png(path, &frame.directional_shadow, None, layer),
        CaptureBuffer::SpotShadow => write_depth_png(path, &frame.spot_shadow, None, 0),
        CaptureBuffer::PointShadow(layer) => write_depth_png(path, &frame.point_shadow, None, layer),
    }
}

fn write_depth_png(path: &Path, buffer: &CapturedDepthBuffer, camera_clip: Option<(f32, f32)>, layer: usize) -> Result<(), String> {
    let values = buffer.layers.get(layer).ok_or_else(|| format!("requested render buffer layer {layer} is not available in this scene"))?;
    let mut pixels = Vec::with_capacity((buffer.width * buffer.height * 4) as usize);
    for value in values.iter().take((buffer.width * buffer.height) as usize) {
        let depth = if let Some((near, far)) = camera_clip {
            let near = near.max(0.001);
            let far = far.max(near + 0.001);
            let depth = value.clamp(0.0, 1.0);
            let linear_distance = near * far / (far - depth * (far - near));
            ((linear_distance - near) / (far - near)).clamp(0.0, 1.0)
        } else if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let intensity = ((1.0 - depth).powf(0.35) * 255.0).round() as u8;
        pixels.extend_from_slice(&[intensity, intensity, intensity, 255]);
    }
    write_rgba_png(path, buffer.width, buffer.height, &pixels)
}

fn write_rgba_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|error| format!("failed to create MCP capture: {error}"))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| format!("failed to write MCP PNG header: {error}"))?;
    writer.write_image_data(pixels).map_err(|error| format!("failed to write MCP PNG pixels: {error}"))
}
