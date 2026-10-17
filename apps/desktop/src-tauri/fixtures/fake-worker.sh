#!/bin/sh
# Explicitly compiled private fixture only. No recognition engine, network or media playback.
[ "$*" = '--protocol 1' ] || exit 70
input=$(/bin/cat)
id=$(printf '%s' "$input" | /usr/bin/sed -En 's/.*"task_id":"([^"]+)".*/\1/p')
attempt=$(printf '%s' "$input" | /usr/bin/sed -En 's/.*"attempt":([0-9]+).*/\1/p')
printf '{"type":"result","schema_version":1,"task_id":"%s","attempt":%s,"transcript":{"schemaVersion":1,"modelVersion":"fixture-asr","durationMs":1000,"segments":[{"source":"mic","startMs":0,"endMs":900,"text":"PRIVATE_SYNTHETIC_TRANSCRIPT"}]}}\n' "$id" "$attempt"
