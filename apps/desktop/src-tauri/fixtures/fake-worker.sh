#!/bin/sh
# Explicit native-bridge-fixture/test only; Python is existing fixture tooling,
# not a production ASR runtime. No engine, network, audio decoding or playback.
[ "$#" -eq 2 ] && [ "$1" = '--protocol' ] && [ "$2" = '1' ] || exit 70
exec /usr/bin/python3 -I -B -S -c '
import json
import sys
import uuid

U64 = (1 << 64) - 1

def uint(value, maximum=U64):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError("invalid integer")
    return value

def identifier(value):
    return str(uuid.UUID(value))

def digest(value):
    if (not isinstance(value, str) or len(value) != 64
        or any(character not in "0123456789abcdefABCDEF" for character in value)):
        raise ValueError("invalid digest")

def object_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate field")
        result[key] = value
    return result

try:
    data = sys.stdin.buffer.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("request exceeds protocol limit")
    envelope = json.loads(data, object_pairs_hook=object_fields)
    if uint(envelope["schema_version"]) != 1:
        raise ValueError("unsupported schema")
    request = envelope["request"]
    context = request["context"]
    task = identifier(context["task_id"])
    session = identifier(context["session_id"])
    attempt = uint(context["attempt"], (1 << 32) - 1)
    segments = request["segments"]
    if not isinstance(segments, list) or not segments:
        raise ValueError("missing segments")
    rolling = any(segment.get("capture") is not None for segment in segments)
    first = segments[0].get("capture") if rolling else None
    origin = uint(first["offset_frames"]) if rolling else 0
    previous = origin
    duration = 0
    for index, segment in enumerate(segments):
        if not isinstance(segment["path"], str) or not segment["path"]:
            raise ValueError("missing audio path")
        digest(segment["sha256"])
        milliseconds = uint(segment["duration_ms"])
        if not milliseconds:
            raise ValueError("empty interval")
        if rolling:
            capture = segment["capture"]
            frames = uint(capture["frame_count"])
            offset = uint(capture["offset_frames"])
            scaled_frames = uint(frames * 1000)
            if (not frames or uint(capture["sample_rate"]) != 48000
                or offset != previous or scaled_frames // 48000 != milliseconds
                or identifier(capture["session_id"]) != session
                or identifier(capture["instance_id"]) != identifier(first["instance_id"])
                or uint(capture["sequence"], (1 << 32) - 1)
                   != uint(first["sequence"] + index, (1 << 32) - 1)):
                raise ValueError("inconsistent rolling interval")
            digest(capture["descriptor_sha256"])
            previous = uint(offset + frames)
            # Match audio_ranges_ms: subtract the capture origin and round the
            # cumulative frame boundary, never sum individually rounded lengths.
            duration = uint((previous - origin) * 1000) // 48000
        else:
            duration = uint(duration + milliseconds)
    result = {
        "type": "result", "schema_version": 1, "task_id": task, "attempt": attempt,
        "transcript": {
            "schemaVersion": 1, "modelVersion": "fixture-asr", "durationMs": duration,
            "segments": [{"source": "mic", "startMs": 0, "endMs": min(900, duration),
                          "text": "PRIVATE_SYNTHETIC_TRANSCRIPT"}]
        }
    }
except (KeyError, TypeError, ValueError, AttributeError, OverflowError):
    sys.stderr.write("invalid private fixture ASR request\n")
    sys.exit(70)
print(json.dumps(result, separators=(",", ":")))
'
