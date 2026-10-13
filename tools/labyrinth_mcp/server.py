"""Read-only MCP bridge for a running Labyrinth Engine debug endpoint."""

import base64
import json
import os
import socket
import struct
import sys


BUFFER_NAMES = [
    "color",
    "depth",
    "normals",
    "directional_shadow_0",
    "directional_shadow_1",
    "directional_shadow_2",
    "spot_shadow",
    "point_shadow_0",
    "point_shadow_1",
    "point_shadow_2",
    "point_shadow_3",
    "point_shadow_4",
    "point_shadow_5",
    "point_shadow_6",
    "point_shadow_7",
    "point_shadow_8",
    "point_shadow_9",
    "point_shadow_10",
    "point_shadow_11",
    "unlit",
    "wireframe",
    "shadow_visibility",
    "gi_only",
    "gi_support",
]


def engine_address():
    host = os.environ.get("LABYRINTH_MCP_HOST", "127.0.0.1")
    if host not in ("127.0.0.1", "localhost", "::1"):
        raise ValueError("LABYRINTH_MCP_HOST must point to the local machine")
    port = int(os.environ.get("LABYRINTH_MCP_PORT", "43856"))
    if not 1 <= port <= 65535:
        raise ValueError("LABYRINTH_MCP_PORT must be between 1 and 65535")
    return host, port


def capture(buffer, width=None, height=None):
    image, image_width, image_height, _, _ = capture_with_frame(buffer, width, height)
    return image, image_width, image_height


def capture_with_frame(buffer, width=None, height=None):
    if buffer not in BUFFER_NAMES:
        raise ValueError(f"unknown render buffer '{buffer}'")
    if (width is None) != (height is None):
        raise ValueError("width and height must be supplied together")
    if width is None:
        width = height = 0
    elif width <= 0 or height <= 0:
        raise ValueError("width and height must be positive integers")
    for label, value in (("width", width), ("height", height)):
        if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 4096:
            raise ValueError(f"{label} must be an integer from 1 to 4096, or omitted to use the current window size")
    if (width == 0) != (height == 0):
        raise ValueError("width and height must be supplied together")

    image, frame_id, paused = engine_request_with_frame(f"capture {buffer} {width} {height}")
    image_width, image_height = struct.unpack(">II", image[16:24])
    return image, image_width, image_height, frame_id, paused


def engine_request(request, timeout=35):
    payload, _, _ = engine_request_with_frame(request, timeout)
    return payload


def engine_request_with_frame(request, timeout=35):
    host, port = engine_address()
    with socket.create_connection((host, port), timeout=timeout) as connection:
        connection.settimeout(timeout)
        connection.sendall(request.encode("ascii") + b"\n")
        response = connection.makefile("rb")
        header = response.readline(512).decode("utf-8", errors="replace").rstrip("\r\n")
        fields = header.split("\t")
        if len(fields) < 2:
            raise RuntimeError("engine returned an invalid response")
        if fields[0] != "OK":
            raise RuntimeError(fields[1] if len(fields) > 1 else "engine capture failed")
        image_size = int(fields[1])
        frame_id = int(fields[2]) if len(fields) > 2 else None
        paused = fields[3].lower() == "true" if len(fields) > 3 else False
        if image_size <= 0 or image_size > 128 * 1024 * 1024:
            raise RuntimeError("engine returned an invalid image size")
        payload = response.read(image_size)
        if len(payload) != image_size:
            raise RuntimeError("engine closed the connection before the response was complete")
        return payload, frame_id, paused


def inspect_grids():
    return json.loads(engine_request("grid_info"))


def pick_viewport_pixel(x, y, width=None, height=None):
    if (width is None) != (height is None):
        raise ValueError("width and height must be supplied together")
    if width is None:
        width = height = 0
    if not all(isinstance(value, int) and not isinstance(value, bool) for value in (x, y, width, height)):
        raise ValueError("pixel and viewport dimensions must be integers")
    if x < 0 or y < 0 or width < 0 or height < 0 or width > 4096 or height > 4096:
        raise ValueError("pixel and viewport dimensions are out of range")
    if (width == 0) != (height == 0) or x > 4096 or y > 4096:
        raise ValueError("supply both viewport dimensions, or omit both to use the current viewport")
    return json.loads(engine_request(f"pick {x} {y} {width} {height}"))


def inspect_radiance_probe(x, y, z):
    if not all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in (x, y, z)):
        raise ValueError("probe coordinates must be non-negative integers")
    return json.loads(engine_request(f"probe {x} {y} {z}"))


def inspect_sdf_voxel(x, y, z):
    if not all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in (x, y, z)):
        raise ValueError("SDF voxel coordinates must be non-negative integers")
    return json.loads(engine_request(f"sdf_voxel {x} {y} {z}"))


def pause_engine():
    return json.loads(engine_request("pause"))


def resume_engine():
    return json.loads(engine_request("resume"))


def get_engine_status():
    return json.loads(engine_request("status"))


def get_performance_stats():
    return json.loads(engine_request("performance_stats"))


def profile_frames(frame_count=60, include_gpu_timing=False):
    if not isinstance(frame_count, int) or isinstance(frame_count, bool) or not 1 <= frame_count <= 120:
        raise ValueError("frame_count must be an integer from 1 to 120")
    if not isinstance(include_gpu_timing, bool):
        raise ValueError("include_gpu_timing must be a boolean")
    request = f"profile {frame_count} {str(include_gpu_timing).lower()}"
    return json.loads(engine_request(request, timeout=185))


def tool_result(content, is_error=False):
    return {"content": content, "isError": is_error}


def handle_tool(name, arguments):
    arguments = arguments or {}
    if name == "list_render_buffers":
        return tool_result([{"type": "text", "text": "Available buffers: " + ", ".join(BUFFER_NAMES)}])
    if name == "inspect_grids":
        return tool_result([{"type": "text", "text": json.dumps(inspect_grids(), indent=2)}])
    if name == "pause_engine":
        return tool_result([{"type": "text", "text": json.dumps(pause_engine(), indent=2)}])
    if name == "resume_engine":
        return tool_result([{"type": "text", "text": json.dumps(resume_engine(), indent=2)}])
    if name == "get_engine_status":
        return tool_result([{"type": "text", "text": json.dumps(get_engine_status(), indent=2)}])
    if name == "get_performance_stats":
        return tool_result([{"type": "text", "text": json.dumps(get_performance_stats(), indent=2)}])
    if name == "profile_frames":
        result = profile_frames(arguments.get("frame_count", 60), arguments.get("include_gpu_timing", False))
        return tool_result([{"type": "text", "text": json.dumps(result, indent=2)}])
    if name == "pick_viewport_pixel":
        result = pick_viewport_pixel(arguments.get("x"), arguments.get("y"), arguments.get("width"), arguments.get("height"))
        return tool_result([{"type": "text", "text": json.dumps(result, indent=2)}])
    if name == "inspect_radiance_probe":
        result = inspect_radiance_probe(arguments.get("x"), arguments.get("y"), arguments.get("z"))
        return tool_result([{"type": "text", "text": json.dumps(result, indent=2)}])
    if name == "inspect_sdf_voxel":
        result = inspect_sdf_voxel(arguments.get("x"), arguments.get("y"), arguments.get("z"))
        return tool_result([{"type": "text", "text": json.dumps(result, indent=2)}])
    if name != "capture_render_buffer":
        raise ValueError(f"unknown tool '{name}'")
    image, width, height, frame_id, paused = capture_with_frame(arguments.get("buffer"), arguments.get("width"), arguments.get("height"))
    dimensions = f"{width}x{height}" if width and height else "current window size"
    return tool_result([
        {"type": "text", "text": f"Captured '{arguments['buffer']}' at {dimensions} from frame {frame_id} (paused={str(paused).lower()})."},
        {"type": "image", "mimeType": "image/png", "data": base64.b64encode(image).decode("ascii")},
    ])


TOOLS = [
    {
        "name": "pause_engine",
        "description": "Pause the application after its current completed frame. The app and renderer stop advancing while MCP inspection and captures continue. Returns the frozen frame ID.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "resume_engine",
        "description": "Resume application updates and rendering after MCP analysis. Returns the last completed frame ID.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "get_engine_status",
        "description": "Read whether the application is paused and the ID of its last completed frame.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "get_performance_stats",
        "description": "Read detailed CPU phase timings, GPU pass timings, GI workload counters, shadow scheduling, and triangle-BVH diagnostics for the latest completed frame. This performs a one-shot GPU timestamp readback and may wait for the GPU.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "profile_frames",
        "description": "Collect per-frame CPU phase timings and renderer work counters for a bounded sequence of upcoming frames. The engine must be running. GPU timestamp readback is optional and can perturb frame pacing because it synchronizes with the GPU.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "frame_count": {"type": "integer", "minimum": 1, "maximum": 120, "default": 60, "description": "Number of consecutive frames to sample."},
                "include_gpu_timing": {"type": "boolean", "default": False, "description": "Read GPU timestamps for every sampled frame; this synchronizes with the GPU and can perturb performance."},
            },
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "inspect_grids",
        "description": "Read SDF and world-radiance grid bounds, resolutions, cell spacing, and update counters. The result includes frame_id and paused metadata; pause_engine first to keep related queries on one frame.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "pick_viewport_pixel",
        "description": "Pick the nearest scene triangle at a pixel from the current camera viewport. Returns world position, normal, instance and triangle indices, containing SDF/radiance cell coordinates, and frame metadata. Pause first to align the pick with a screenshot.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "x": {"type": "integer", "minimum": 0, "description": "Horizontal screenshot pixel coordinate."},
                "y": {"type": "integer", "minimum": 0, "description": "Vertical screenshot pixel coordinate."},
                "width": {"type": "integer", "minimum": 1, "maximum": 4096, "description": "Screenshot width, if it differs from the current viewport."},
                "height": {"type": "integer", "minimum": 1, "maximum": 4096, "description": "Screenshot height, if it differs from the current viewport."},
            },
            "required": ["x", "y"],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "inspect_radiance_probe",
        "description": "Read one world-radiance field cell, including relocated probe position, validity, angular radiance samples, sample and attempt counts, freshness age, coverage summary, and frame metadata.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "x": {"type": "integer", "minimum": 0, "description": "Radiance field cell X coordinate."},
                "y": {"type": "integer", "minimum": 0, "description": "Radiance field cell Y coordinate."},
                "z": {"type": "integer", "minimum": 0, "description": "Radiance field cell Z coordinate."},
            },
            "required": ["x", "y", "z"],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "inspect_sdf_voxel",
        "description": "Read one SDF grid voxel, including signed distance, surface albedo, emission, conservative clearance, and frame metadata.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "x": {"type": "integer", "minimum": 0, "description": "SDF voxel X coordinate."},
                "y": {"type": "integer", "minimum": 0, "description": "SDF voxel Y coordinate."},
                "z": {"type": "integer", "minimum": 0, "description": "SDF voxel Z coordinate."},
            },
            "required": ["x", "y", "z"],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "list_render_buffers",
        "description": "List the render buffers and renderer debug views that can be captured from the running Labyrinth Engine project.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
    {
        "name": "capture_render_buffer",
        "description": "Capture a PNG from the running project's active camera. The response identifies its frame and pause state. Pause first to align it with picks and grid queries.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "buffer": {"type": "string", "enum": BUFFER_NAMES, "description": "The render target or renderer debug view to capture."},
                "width": {"type": "integer", "minimum": 1, "maximum": 4096},
                "height": {"type": "integer", "minimum": 1, "maximum": 4096},
            },
            "required": ["buffer"],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False},
    },
]


def handle_message(message):
    if not isinstance(message, dict):
        return None
    method = message.get("method")
    request_id = message.get("id")
    if request_id is None:
        return None
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "labyrinth-engine", "version": "0.1.0"},
                "instructions": "Use pause_engine to freeze the application after a completed frame, inspect or capture data while paused, then call resume_engine. Use profile_frames while the engine is running; GPU timestamp collection synchronizes with the GPU. Responses include frame_id metadata. Start the project with LABYRINTH_MCP_PORT=43856 set.",
            },
        }
    if method == "ping":
        return {"jsonrpc": "2.0", "id": request_id, "result": {}}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        try:
            params = message.get("params", {})
            result = handle_tool(params.get("name"), params.get("arguments"))
            return {"jsonrpc": "2.0", "id": request_id, "result": result}
        except Exception as error:
            return {"jsonrpc": "2.0", "id": request_id, "result": tool_result([{"type": "text", "text": str(error)}], True)}
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": -32601, "message": f"method not found: {method}"},
    }


def main():
    for line in sys.stdin:
        try:
            message = json.loads(line)
            response = handle_message(message)
        except Exception as error:
            response = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(error)}}
        if response is not None:
            sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
