"""Read and verify a completed fixture stage's host evidence; never launch a process."""
import json
import pathlib

from fixture_runner import checked_path, fixture_build_spec, sha


def host_artifacts(root, stage, revision, scenario):
    root = checked_path(pathlib.Path(root))
    stage = checked_path(pathlib.Path(stage))
    assert stage.is_dir() and stage.is_relative_to('/private/tmp'), 'private fixture stage required'
    provenance_path = checked_path(stage / 'runner-provenance.json')
    provenance = json.loads(provenance_path.read_text())
    assert isinstance(provenance, dict), 'invalid runner provenance'
    assert provenance.get('sourceRevision') == revision, 'fixture source revision mismatch'
    assert provenance.get('scenario') == scenario, 'fixture scenario mismatch'
    argv = provenance.get('argv')
    assert isinstance(argv, list) and argv and isinstance(argv[0], str), 'missing fixture host argv'
    actual = pathlib.Path(argv[0])
    assert actual == stage / 'Fixture.app/Contents/MacOS/omm-desktop', 'host outside this fixture stage'
    _, expected_source = fixture_build_spec(root)
    assert provenance.get('fixtureSourceHost') == str(expected_source), 'fixture source outside this build'
    source = pathlib.Path(provenance['fixtureSourceHost'])
    actual_hash = sha(checked_path(actual))
    source_hash = sha(checked_path(source))
    assert actual_hash == provenance.get('actualHostHash'), 'actual fixture host hash mismatch'
    assert source_hash == provenance.get('originalFixtureHostHash'), 'source fixture host hash mismatch'
    start_path = checked_path(stage / 'host-start.json')
    start = json.loads(start_path.read_text())
    assert isinstance(start, dict), 'invalid host start receipt'
    assert start.get('sourceRevision') == revision, 'host start revision mismatch'
    assert start.get('executable') == str(actual) and start.get('sha256') == actual_hash, 'host start identity mismatch'
    return {str(actual): actual_hash, str(source): source_hash,
            str(provenance_path): sha(provenance_path), str(start_path): sha(start_path)}
