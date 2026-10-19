import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('../apps/desktop/src-tauri/src/bridge_fixture/ui_startup.rs', import.meta.url), 'utf8');
const script = source.match(/pub const BOOTSTRAP: &str = r#"([\s\S]*?)"#;/)[1];
const readScript = JSON.parse(source.match(/const READ: &str =\s*("[^\n]+?");/)[1]);
const cancelScript = JSON.parse(source.match(/const CANCEL: &str =\s*("[^\n]+?");/)[1]);
const fixture = () => {
  let id=0, requests=0, cancels=0, calls=0, failRequest=0, failCancel=false;
  const queue=new Map();
  const document={visibilityState:'visible',hasFocus:()=>true};
  Object.defineProperty(document,'body',{get(){throw Error('private body must not be read');}});
  const window={innerWidth:1280,innerHeight:820,
    requestAnimationFrame:callback=>{requests++;if(requests===failRequest)throw Error('private request');queue.set(++id,callback);return id;},
    cancelAnimationFrame:handle=>{cancels++;if(failCancel)throw Error('private cancel');queue.delete(handle);},
    addEventListener:()=>{calls++;throw Error('no listener');},
    fetch:()=>{calls++;throw Error('no request');},
  };
  const context=vm.createContext({window,document,setTimeout:()=>{calls++;throw Error('no timer');}});
  return {window,document,queue,context,
    start:()=>JSON.parse(vm.runInContext(script,context)),
    read:()=>JSON.parse(vm.runInContext(readScript,context)),
    cancel:()=>JSON.parse(vm.runInContext(cancelScript,context)),
    frame:()=>{const [key,callback]=queue.entries().next().value;queue.delete(key);callback();return callback;},
    failRequest:n=>{failRequest=n;},failCancel:()=>{failCancel=true;},
    counts:()=>({requests,cancels,calls})};
};

test('compiled bootstrap needs two serial natural callbacks, never drives business or timers',()=>{
  const f=fixture();assert.equal(f.start().frames,0);assert.equal(f.queue.size,1);
  for(let i=0;i<3;i++)assert.equal(f.read().frames,0);
  f.frame();assert.equal(f.read().frames,1);assert.equal(f.queue.size,1);
  f.frame();assert.equal(f.read().frames,2);assert.equal(f.queue.size,0);
  assert.deepEqual(f.counts(),{requests:2,cancels:0,calls:0});
  assert.deepEqual(Object.keys(f.read()).sort(),['cleanup','error','focused','frames','viewport','visible']);
  assert.equal(f.cancel().cleanup,0);
});

test('compiled facts reject hidden, unfocused and nonfinite or nonpositive viewport without content',()=>{
  const f=fixture();f.start();f.frame();f.frame();
  f.document.visibilityState='hidden';assert.equal(f.read().visible,false);
  f.document.visibilityState='visible';f.document.hasFocus=()=>false;assert.equal(f.read().focused,false);
  f.document.hasFocus=()=>true;
  for(const value of [NaN,Infinity,-Infinity,0,-1]){f.window.innerWidth=value;assert.equal(f.read().viewport,false);}
  f.window.innerWidth=800;f.window.innerHeight=600;assert.equal(f.read().viewport,true);
  f.document.hasFocus=()=>{throw Error('/private/secret user content');};const result=f.read();
  assert.equal(result.error,true);assert.equal(JSON.stringify(result).includes('secret'),false);
  assert(Object.values(result).every(v=>typeof v==='boolean'||(Number.isInteger(v)&&v>=0&&v<=2)));
  assert.equal(f.counts().calls,0);
});

test('missing API and first/second registration throws preserve bounded 0/1 counts',()=>{
  for(const missing of ['requestAnimationFrame','cancelAnimationFrame']){const f=fixture();delete f.window[missing];assert.equal(f.start().error,true);assert.equal(f.read().frames,0);assert.equal(f.queue.size,0);}
  const first=fixture();first.failRequest(1);assert.equal(first.start().error,true);assert.equal(first.read().frames,0);
  const second=fixture();second.failRequest(2);second.start();second.frame();assert.equal(second.read().frames,1);assert.equal(second.read().error,true);assert.equal(second.queue.size,0);
});

test('cancel touches only its pending frame, suppresses late callbacks and reports unknown on throws',()=>{
  for(const advance of [false,true]){
    const f=fixture();f.start();if(advance)f.frame();const late=[...f.queue.values()][0];
    f.queue.set(999,()=>{throw Error('not gate owned');});const before=f.read().frames;
    assert.equal(f.cancel().cleanup,1);assert.equal(f.queue.size,1);assert(f.queue.has(999));
    late();assert.equal(f.read().frames,before);assert.equal(f.counts().requests,advance?2:1);
    assert.equal(f.counts().cancels,1);
  }
  const absent=fixture();assert.equal(absent.cancel().cleanup,2);
  const f=fixture();f.start();const late=[...f.queue.values()][0];f.failCancel();assert.equal(f.cancel().cleanup,2);late();assert.equal(f.read().frames,0);
});
