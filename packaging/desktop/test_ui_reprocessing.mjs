import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../../scripts/fixtures/desktop-ui-probe.js', import.meta.url), 'utf8');
const prefix = source.split('// Real React events')[0];
const context = vm.createContext({});
vm.runInContext(prefix, context);
const prove = context.reprocessingEvidence;
assert.equal(typeof prove, 'function');
// Execute the driver's actual response extraction, not a reconstruction in this test.
const assignments = source.split('\n').map(line => line.trim()).filter(line => line.startsWith('const transcript = '));
assert.equal(assignments.length, 1);
const extraction = assignments[0].slice('const transcript = '.length, -1);
const extract = artifact => vm.runInNewContext(extraction, {artifact});
const task = {taskId:'new', stage:'summary'};
const version = {artifact:{taskId:'new',stage:'transcribe'},status:'committed',sourceTranscriptTaskId:null};
// ArtifactChunk from src/application/artifacts.rs: transcripts use lines; text is empty.
const response = {
  artifact:{meetingId:'meeting',taskId:'new',stage:'transcribe'}, revision:'synthetic-revision',
  status:'committed',format:'transcript',text:'',
  lines:[{startMs:0,endMs:1000,speaker:'microphone',text:'PRIVATE_SYNTHETIC_TRANSCRIPT'}],
  nextCursor:null,complete:true,
};
const evidence = (stage, current, versions, artifact = response) => prove(stage, current, versions, extract(artifact));
assert.equal(evidence('transcribe', task, [version]), true, 'real transcript response with empty text and worker lines accepts fast advancement');
assert.equal(evidence('transcribe', task, [{...version,sourceTranscriptTaskId:'old'}]), false, 'summary-only reused transcript rejected');
assert.equal(evidence('transcribe', task, [version], {...response,lines:[{...response.lines[0],text:'Private synthetic transcript token'}]}), false, 'seed transcript without worker execution rejected');
assert.equal(evidence('transcribe', task, [{...version,artifact:{taskId:'other',stage:'transcribe'}}]), false, 'wrong generation rejected');
assert.equal(evidence('transcribe', {...task,stage:'unknown'}, [version]), false, 'undefined stage rejected');
assert.equal(evidence('summary', task, [{...version,sourceTranscriptTaskId:'old'}], null), true, 'summary request retains source');
assert.equal(evidence('summary', {...task,stage:'transcribe'}, [version]), false, 'wrong starting stage rejected');
assert.equal(evidence('transcribe', {...task,stage:'transcribe'}, [{...version,status:'missing'}]), false, 'queued does not prove execution');
assert.equal(evidence('transcribe', task, [version], {...response,text:'PRIVATE_SYNTHETIC_TRANSCRIPT',lines:[]}), false, 'top-level text cannot impersonate transcript lines');
assert.equal(evidence('transcribe', task, [version], null), false, 'absent artifact cannot prove execution');
console.log('10 source-bound artifact-response reprocessing assertions passed');
