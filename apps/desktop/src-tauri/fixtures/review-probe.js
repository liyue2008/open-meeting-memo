(async()=>{
  const checks=[];let sequence=0,playCalls=0;
  HTMLMediaElement.prototype.play=function(){playCalls++;throw Error('play forbidden');};
  const check=(name,ok)=>{if(!ok)throw Error(name);checks.push(name);};
  const wait=async(fn,label)=>{const end=performance.now()+15000;while(performance.now()<end){const v=await fn();if(v)return v;await new Promise(r=>setTimeout(r,100));}throw Error('timeout '+label);};
  const raw=(command,payload)=>window.__TAURI_INTERNALS__.invoke(command,{request:{schemaVersion:1,requestId:`review-probe-${++sequence}`,payload}});
  const call=async(command,payload)=>{const r=await raw(command,payload);if(r.error)throw Error(command+JSON.stringify(r.error));return r.data;};
  const control=async(action)=>{const id=`review-${++sequence}`;window.__ommReviewControlResult=null;window.__ommReviewControl={id,action};const r=await wait(()=>window.__ommReviewControlResult?.id===id&&window.__ommReviewControlResult,action);if(!r.ok)throw Error(JSON.stringify(r));return r.data;};
  const button=text=>Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim().replace(/^＋\s*/,'')===text);
  const set=(node,value)=>{Object.getOwnPropertyDescriptor(node instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(node,value);node.dispatchEvent(new Event(node instanceof HTMLSelectElement?'change':'input',{bubbles:true}));};
  const reopen=async()=>{ /* Published React query freshness is 1 second; remount only once stale. */ await new Promise(r=>setTimeout(r,1100));(await wait(()=>button('← 返回会议库'),'back')).click();(await wait(()=>button('Review real partial transcript'),'partial meeting')).click();(await wait(()=>button('逐字稿'),'transcript tab')).click();};
  try{
    await wait(()=>window.__ommReviewReady,'native controller');
    const invalidId=`review-invalid-${++sequence}`;window.__ommReviewControlResult=null;window.__ommReviewControl={id:invalidId,action:'inspect',path:'/must-not-be-read'};
    check('unknown field rejected',!(await wait(()=>window.__ommReviewControlResult?.id===invalidId&&window.__ommReviewControlResult,'reject fields')).ok);
    const initial=await control('inspect');
    if(initial.scenario.mode==='cold-recording'){
      check('cold socket absent before real React operation',initial.scenario.coldBaseline.helperSocketAbsent&&initial.capturePids.length===0);
      (await wait(()=>button('新建会议'),'new meeting')).click();
      const dialog=await wait(()=>document.querySelector('dialog[open], [role="dialog"]'),'new dialog');
      set(dialog.querySelector('input:not([type="checkbox"]):not([type="datetime-local"])'),'Review cold real React capture');
      check('real consent gate disabled before confirmation',button('开始录音').disabled);
      dialog.querySelector('input[type="checkbox"]').click();
      await wait(()=>!button('开始录音').disabled,'consent enabled');button('开始录音').click();
      await wait(async()=>((await call('record_status',{})).state==='recording'),'real typed recording');
      const started=await control('inspect');
      check('production ensure_helper spawned exactly once',started.capturePids.length===1);
      check('real peer acknowledged capture',started.captureEvents.some(e=>e.action==='bound')&&started.captureEvents.some(e=>e.action==='start'));
      (await wait(()=>button('停止并封存'),'real stop button')).click();
      await wait(async()=>((await call('record_status',{})).state==='idle'),'record sealed');
      const sealed=await control('inspect');
      check('real sealed segment response',sealed.captureEvents.some(e=>e.action==='sealed_segments'&&e.reply.segments.length===1));
      await control('restore');
      check('restore stops only owned idle peer',true);
    }else{
      const artifact=initial.scenario.artifact;
      const premature=`review-premature-${++sequence}`;window.__ommReviewControlResult=null;window.__ommReviewControl={id:premature,action:'finalize'};
      check('out-of-stage finalize rejected',!(await wait(()=>window.__ommReviewControlResult?.id===premature&&window.__ommReviewControlResult,'reject phase')).ok);
      check('real parent initially partial without whole commit',!initial.wholeCommitted&&!!initial.partialRevision);
      const page=await call('artifact_read',{artifact,cursor:null,maxBytes:150});
      check('partial chunk has real cursor',page.status==='partial'&&!!page.nextCursor);
      const date=await wait(()=>Array.from(document.querySelectorAll('select')).find(s=>s.querySelector('option[value="all"]')?.textContent==='全部时间'),'date selector');set(date,'all');
      const search=await wait(()=>document.querySelector('[aria-label="搜索会议"]'),'search');set(search,'Review real partial transcript');
      (await wait(()=>button('Review real partial transcript'),'partial meeting')).click();
      (await wait(()=>button('逐字稿'),'transcript tab')).click();
      await wait(()=>document.body.innerText.includes('当前内容不是终稿')&&document.body.innerText.includes('Review partial segment 0'),'real partial UI');
      check('real React renders partial notice and verified lines',true);
      const advanced=await control('advance');
      check('real segment commit advances partial revision',advanced.partialRevision!==initial.partialRevision&&!advanced.wholeCommitted);
      check('old partial cursor rejected',(await raw('artifact_read',{artifact,cursor:page.nextCursor,maxBytes:150})).error?.code==='conflict');
      await reopen();await wait(()=>document.body.innerText.includes('Review partial segment 1'),'updated partial UI');
      check('real React reloads second verified segment',true);
      const finalized=await control('finalize');
      check('real whole transcript commit replaces partial',finalized.wholeCommitted&&!finalized.partialRevision);
      await reopen();await wait(()=>document.body.innerText.includes('Review partial segment 1')&&!document.body.innerText.includes('当前内容不是终稿'),'final UI');
      check('real React switches to final artifact',true);
      (await wait(()=>button('← 返回会议库'),'release audio reader')).click();
    }
    await control('finish');
    window.__ommFixtureResult={ok:true,checks,playCalls};
  }catch(error){window.__ommFixtureResult={ok:false,checks,playCalls,error:String(error),dom:document.body.innerText.slice(0,5000)};}
})();
