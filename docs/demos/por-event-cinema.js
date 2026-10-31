/* Read-only replay of actual harness events. Motion is presentation, not timing evidence. */
(() => {
  const C = id => document.getElementById('cinema-' + id);
  const types = new Set(['run_started','equivocation_verified','penalty_verified','block_added','checkpoint_started','ratings_staged','closure_checked','peer_batch_published','excluded_batch_published','checkpoint_closed','observer_committed','round_completed','observers_restarted','run_completed']);
  const names = ['validator-1','validator-2','validator-3','validator-4'];
  let events=[], position=-1, playing=null, connected=false, pollTimer=null, generation=0, cursor=0, runId='', lastArrival=0, failureCount=0;
  let state, reduced=matchMedia('(prefers-reduced-motion: reduce)').matches;
  const text=(id,value)=>{C(id).textContent=value;};
  const msg=value=>text('message',value);
  const hash=value=>typeof value==='string'&&/^[0-9a-f]{64}$/i.test(value);
  const numeric=value=>Number.isSafeInteger(value)&&value>=0;
  function validate(e, previous) {
    if(!e||e.schema_version!==1||e.kind!=='por_event'||!types.has(e.event)||!numeric(e.seq)||!numeric(e.round)||!numeric(e.elapsed_ms)||!e.data||typeof e.data!=='object'||!['honest','unequal','isolated','correlated','example'].includes(e.scenario)||typeof e.run_id!=='string'||e.run_id.length>200) throw Error('Invalid event record.');
    if(previous&&(e.seq!==previous.seq+1||e.run_id!==previous.run_id||e.scenario!==previous.scenario||e.elapsed_ms<previous.elapsed_ms)) throw Error('Events must belong to one run in contiguous sequence and time order.');
    if(!previous&&(e.seq!==0||e.event!=='run_started')) throw Error('The trace must begin with run_started at sequence 0.');
    if(e.data.observer!=null&&(!Number.isInteger(e.data.observer)||e.data.observer<1||e.data.observer>4)) throw Error('Invalid observer identity.');
    if(e.data.peer!=null&&(!Number.isInteger(e.data.peer)||e.data.peer<1||e.data.peer>4)) throw Error('Invalid peer identity.');
    if(['ratings_staged','closure_checked','peer_batch_published','excluded_batch_published','checkpoint_closed','observer_committed','penalty_verified'].includes(e.event)&&!Number.isInteger(e.data.observer)) throw Error('Observer event is missing its identity.');
    if(['peer_batch_published','excluded_batch_published'].includes(e.event)&&(!Number.isInteger(e.data.peer)||!numeric(e.data.count)||!numeric(e.data.copies_per_rating))) throw Error('Invalid peer batch.');
    if(e.event==='equivocation_verified'&&(!Array.isArray(e.data.blocks)||e.data.blocks.length>16||!e.data.blocks.every(b=>hash(b.id)&&Array.isArray(b.parents)&&b.parents.every(hash)))) throw Error('Invalid proof blocks.');
    if(e.event==='penalty_verified'&&(!names.includes(e.data.validator)||!['prior','offending_weight','total_weight','penalty','scale','retained'].every(k=>numeric(e.data[k]))||!e.data.scale||!e.data.total_weight)) throw Error('Invalid penalty record.');
    if(e.event==='run_started'&&(!Array.isArray(e.data.initial_weights)||e.data.initial_weights.length!==4||!e.data.initial_weights.every(numeric))) throw Error('Invalid initial weights.');
    if(e.event==='block_added'&&(!hash(e.data.id)||!numeric(e.data.height)||!names.includes(e.data.creator)||!Array.isArray(e.data.parents)||e.data.parents.length>64||!e.data.parents.every(hash))) throw Error('Invalid DAG block.');
    if(['round_completed','observer_committed','checkpoint_started'].includes(e.event)&&(!e.data.weights||!names.every(k=>numeric(e.data.weights[k])))) throw Error('Invalid weight snapshot.');
    if(e.event==='round_completed'&&e.data.entries&&(!Array.isArray(e.data.entries)||!e.data.entries.every(x=>x&&names.includes(x.validator)&&numeric(x.reputation)&&numeric(x.retained_reputation)&&typeof x.excluded==='boolean'))) throw Error('Invalid reputation entries.');
    return e;
  }
  function pause(){clearInterval(playing);playing=null;text('play','Play');}
  function disconnect(){connected=false;generation++;clearTimeout(pollTimer);text('connect','Connect live');}
  function emptyState(){return {blocks:new Map(),round:0,window:1,observers:Array.from({length:4},()=>({status:'Not processed',round:0,count:0})),weights:Object.fromEntries(names.map(k=>[k,0])),retained:{},excluded:[],history:[],proofs:[],penalties:[],latest:null,finished:false,provenance:'instrumented'};}
  function apply(e){
    const d=e.data;state.latest=e;
    switch(e.event){
      case 'run_started':state.weights=Object.fromEntries(names.map((k,i)=>[k,d.initial_weights[i]]));state.provenance=d.provenance||'instrumented';break;
      case 'equivocation_verified':state.proofs.push({...d,checkpoint_round:e.round});break;
      case 'penalty_verified':state.penalties.push({...d,round:e.round});break;
      case 'block_added':state.blocks.set(d.id,d);state.window=Math.floor(d.height/16)+1;break;
      case 'checkpoint_started':state.round=e.round;state.window=e.round;state.weights={...d.weights};state.observers.forEach(o=>{o.status='Awaiting ratings';o.count=0;});break;
      case 'ratings_staged':Object.assign(state.observers[d.observer-1],{status:d.ejected?'Excluded · auditing':'Local ratings staged',count:d.count,round:e.round});break;
      case 'closure_checked':state.observers[d.observer-1].status=d.complete?'Required ratings complete':'Waiting for peers';break;
      case 'peer_batch_published':state.observers[d.observer-1].status='Batch from V'+d.peer;state.observers[d.observer-1].count+=d.count;break;
      case 'excluded_batch_published':state.observers[d.observer-1].status='Excluded-key batch present';break;
      case 'checkpoint_closed':state.observers[d.observer-1].status='Checkpoint closed';break;
      case 'observer_committed':Object.assign(state.observers[d.observer-1],{status:'Committed',round:e.round,hash:d.checkpoint,weights:d.weights});break;
      case 'round_completed':state.round=e.round;state.window=e.round;state.weights={...d.weights};state.history.push(d);state.retained=Object.fromEntries((d.entries||[]).map(x=>[x.validator,x.retained_reputation]));state.excluded=(d.entries||[]).filter(x=>x.excluded).map(x=>x.validator);if(state.provenance==='snapshots')state.observers.forEach(o=>{o.status=d.observer_agreement?'Agreement reported':'Not recorded';o.round=e.round;});break;
      case 'observers_restarted':state.observers.forEach(o=>{o.status='Recovered · state verified';});break;
      case 'run_completed':state.finished=d.status==='passed';break;
    }
  }
  function seek(next){
    next=Math.max(-1,Math.min(events.length-1,next));
    if(!state||next<position){state=emptyState();position=-1;}
    for(let n=position+1;n<=next;n++)apply(events[n]);position=next;draw();
  }
  function load(incoming){
    if(!incoming.length||incoming.length>100000)throw Error('Expected 1–100,000 events.');
    incoming.forEach((e,i)=>validate(e,incoming[i-1]));pause();disconnect();events=incoming;state=emptyState();position=-1;seek(0);
    msg('Loaded '+events.length.toLocaleString()+' events. '+(events.at(-1).event==='run_completed'?'Completion marker recorded.':'Partial trace: no completion marker.'));
  }
  function fromSnapshots(run){
    if(!run?.rows?.length)throw Error('First import or select a result in the explorer.');
    const initial=names.map(k=>run.rows[0].weights[k]??0), id='snapshot-'+Date.now();
    const output=[];const push=(event,round,data)=>output.push({schema_version:1,kind:'por_event',run_id:id,scenario:run.scenario||selected,seq:output.length,elapsed_ms:output.length*1000,event,round,data});
    push('run_started',0,{initial_weights:initial,provenance:'snapshots',planned_rounds:run.rows.length});
    run.rows.forEach(row=>push('round_completed',row.round,row));
    // Preserve only the source file's explicit pass claim, not a synthetic test pass.
    if(run.status==='passed')push('run_completed',run.rows.at(-1).round,{status:'passed',reported_by_file:true});
    load(output);msg('Outcome-only playback. No DAG, exchanges, or processing timestamps were exported. Timeline timing is illustrative; initial display uses the first snapshot.');
  }
  function play(){
    if(playing)return pause();if(!events.length)return;
    window.pausePorReplay?.();stop();if(position>=events.length-1&&!connected)seek(0);
    text('play','Pause');playing=setInterval(()=>{if(position<events.length-1)seek(position+1);else if(!connected)pause();},Number(C('speed').value));
  }
  async function poll(token){
    if(!connected||token!==generation)return;
    try {
      const response=await fetch('/api/por-events?scenario='+encodeURIComponent(C('scenario').value)+'&cursor='+cursor+'&run_id='+encodeURIComponent(runId),{cache:'no-store'});
      if(!response.ok)throw Error('Event endpoint unavailable. Start serve-por-replay.py instead of the plain HTTP server.');
      const page=await response.json();if(token!==generation)return;
      if(!Array.isArray(page.events))throw Error(page.error||'Invalid event response.');
      if(page.reset){events=[];state=emptyState();position=-1;runId=page.run_id;}
      let previous=events.at(-1);for(const e of page.events){validate(e,previous);previous=e;}
      if(events.length+page.events.length>100000)throw Error('Event limit reached. Save this trace and disconnect.');
      events.push(...page.events);cursor=page.cursor;runId=page.run_id||'';failureCount=0;
      if(page.events.length)lastArrival=Date.now();
      if(C('tail').checked&&!playing)seek(events.length-1);else if(position<0&&events.length)seek(0);else draw();
      msg(events.length?'Connected · '+events.length.toLocaleString()+' events received. '+(state?.finished?'Run completion recorded.':lastArrival?'Last event '+Math.floor((Date.now()-lastArrival)/1000)+'s ago. Silence is not a pass or failure.':'Waiting for events.'):'Connected. Waiting for '+C('scenario').value+'.events.jsonl to be created by the test.');
    }catch(error){if(token!==generation)return;failureCount++;msg(error.message);text('status','Feed unavailable');if(failureCount>=3){disconnect();return;}}
    if(connected&&token===generation)pollTimer=setTimeout(()=>poll(token),600);
  }
  function connect(){if(connected){disconnect();draw();return;}pause();events=[];state=emptyState();position=-1;cursor=0;runId='';lastArrival=0;failureCount=0;connected=true;text('connect','Disconnect');poll(++generation);}
  function add(parent,tag,cls,value){const n=el(tag,cls,value);parent.append(n);return n;}
  function arrow(svg,x1,y1,x2,y2,color='#53756b',motion=false){
    svg.append(svgEl('path',{d:`M${x1} ${y1} L${x2} ${y2}`,stroke:color,'stroke-width':motion?3:1.5,fill:'none'}));
    const a=Math.atan2(y2-y1,x2-x1);svg.append(svgEl('path',{d:`M${x2-8*Math.cos(a-.4)} ${y2-8*Math.sin(a-.4)} L${x2} ${y2} L${x2-8*Math.cos(a+.4)} ${y2-8*Math.sin(a+.4)}`,stroke:color,fill:'none'}));
    if(motion){const ball=svgEl('circle',{r:5,fill:color,cx:x2,cy:y2});if(!reduced){ball.removeAttribute('cx');ball.removeAttribute('cy');ball.append(svgEl('animateMotion',{dur:'0.75s',path:`M${x1} ${y1} L${x2} ${y2}`,fill:'freeze'}));}svg.append(ball);}
  }
  function dag(){
    const svg=C('dag');svg.replaceChildren();
    const available=Array.from(state.blocks.values());
    C('window').max=available.reduce((m,b)=>Math.max(m,Math.floor(b.height/16)+1),Math.max(1,state.window));
    if(C('follow').checked)C('window').value=state.window;
    const start=(Math.max(1,Number(C('window').value)||1)-1)*16, span=Number(C('zoom').value);
    const blocks=available.filter(b=>b.height>=start&&b.height<start+span).sort((a,b)=>a.height-b.height);
    names.forEach((name,i)=>{const y=48+i*53;svg.append(svgEl('text',{x:12,y:y+5,class:'cine-lane'},'V'+(i+1)));svg.append(svgEl('path',{d:`M50 ${y} H1080`,stroke:'#26463e','stroke-dasharray':'3 6'}));});
    const coords=new Map(blocks.map(b=>[b.id,{x:75+(b.height-start)/Math.max(1,span-1)*970,y:48+names.indexOf(b.creator)*53}]));
    blocks.forEach(b=>{const end=coords.get(b.id);b.parents.forEach(id=>{const parent=coords.get(id);if(parent)arrow(svg,parent.x+13,parent.y,end.x-13,end.y);else arrow(svg,48,end.y,end.x-13,end.y,'#35594e');});});
    blocks.forEach(b=>{
      const p=coords.get(b.id),g=svgEl('g',{role:'button',tabindex:0,'aria-label':'Block '+b.height+', '+b.creator});
      const active=state.latest.event==='block_added'&&state.latest.data.id===b.id;
      g.append(svgEl('circle',{cx:p.x,cy:p.y,r:active?16:12,fill:b.authorization_count?'#d99765':active?'#bcebc6':'#659e85',stroke:active?'#fff':'#183d33','stroke-width':2}));
      g.append(svgEl('text',{x:p.x,y:p.y+4,'text-anchor':'middle',class:'cine-block-label'},String(b.height)));
      g.append(svgEl('title',{},b.id));
      const inspect=()=>text('block-detail',b.creator+' · height '+b.height+' · '+b.id+' · parents: '+(b.parents.join(', ')||'none')+' · authorization carriers: '+(b.authorization_count||0));
      g.addEventListener('click',inspect);g.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();inspect();}});svg.append(g);
    });
    if(!blocks.length)svg.append(svgEl('text',{x:550,y:120,'text-anchor':'middle',class:'cine-lane'},state.provenance==='snapshots'?'No block trace in this result file':'No blocks recorded in this window yet'));
    text('dag-caption',available.length+' actual fixture blocks captured · displaying heights '+start+'–'+(start+span-1)+'. Green: ordinary block. Amber: authorization carrier. Arrows follow recorded parent links.');
  }
  function network(){
    const svg=C('network');svg.replaceChildren();const points=[{x:100,y:65},{x:500,y:65},{x:100,y:220},{x:500,y:220}];
    const e=state.latest,d=e.data;
    if(['peer_batch_published','excluded_batch_published'].includes(e.event)){const from=points[d.peer-1],to=points[d.observer-1];arrow(svg,from.x,from.y,to.x,to.y,'#e4b375',true);svg.append(svgEl('text',{x:300,y:140,'text-anchor':'middle',class:'cine-lane'},d.count+' ratings × '+d.copies_per_rating+' copies'));}
    else {svg.append(svgEl('text',{x:300,y:125,'text-anchor':'middle',class:'cine-center'},'CHECKPOINT '+state.round));svg.append(svgEl('text',{x:300,y:147,'text-anchor':'middle',class:'cine-lane'},e.event==='observers_restarted'?'Recovery verified':'Independent durable stores'));}
    points.forEach((p,i)=>{
      const o=state.observers[i],active=d.observer===i+1;
      svg.append(svgEl('rect',{x:p.x-88,y:p.y-33,width:176,height:66,rx:12,fill:active?'#284e40':'#162e28',stroke:active?'#b5dfac':'#416355','stroke-width':active?2:1}));
      svg.append(svgEl('text',{x:p.x,y:p.y-10,'text-anchor':'middle',class:'cine-observer'},'Observer '+(i+1)));
      svg.append(svgEl('text',{x:p.x,y:p.y+10,'text-anchor':'middle',class:'cine-lane'},o.status));
      svg.append(svgEl('text',{x:p.x,y:p.y+26,'text-anchor':'middle',class:'cine-tiny'},'Round '+o.round+' · '+(state.provenance==='snapshots'?'exchange not traced':o.count+' ratings published/staged')));
    });
  }
  const descriptions={
    equivocation_verified:['Conflicting signed blocks are verified','The fixture verified these actual proof blocks before constructing the authorization carrier. They share a validator identity; they are not inserted as a fabricated fork in the shared fixture chain.'],
    penalty_verified:['The penalty calculation is verified','After commit, the harness checks the exact penalty against the pre-shock snapshot, including offending weight, total active weight, and retained reputation. This event records that verification, not a second slash.'],
    run_started:['Run initialized','The harness opened an event stream. Initial scores are recorded before fixture construction.'],
    block_added:['A signed block enters the fixture DAG','The actual block identity and parent links were captured after insertion. The fixture is built before checkpoint processing; this is not live consensus finalization.'],
    checkpoint_started:['A checkpoint window begins','All observers start from the preceding round. The DAG camera follows the actual 16-block fixture window.'],
    ratings_staged:['Local ratings are staged','The observer has persisted its signed local batch. Excluded observers stage no ratings but may continue to audit.'],
    closure_checked:['Checking for missing ratings','A complete result means all required ratings are present. Zero-length batches need no delivery. An incomplete result keeps this checkpoint open.'],
    peer_batch_published:['A peer batch reaches the file exchange','The harness published this batch into the selected observer’s exchange, twice per rating to exercise duplicate handling. The animated flight illustrates delivery; its duration is not measured network latency.'],
    excluded_batch_published:['An excluded key publishes stale ratings','The harness deliberately writes a signed batch using stale reputation. The checkpoint exchange filters excluded keys; publication is not acceptance. The next closure uses only eligible ratings.'],
    checkpoint_closed:['The observer closes its checkpoint','The complete signed batch was accepted by the checkpoint collector. Closure precedes the durable reputation commit.'],
    observer_committed:['One observer commits reputation','This observer has applied the verified checkpoint and penalties to durable state. Others may still be processing. The shared ledger updates only after all observer states are compared.'],
    round_completed:['Observer outcomes agree','The harness checked matching checkpoint hashes and reputation states, then recorded the common outcome. Reputation, exclusion, retained capital, and convergence metrics below come from that snapshot.'],
    observers_restarted:['Durable state survives restart','All four stores were closed, reopened, and compared with their prior state. The stream records verified recovery, not the internal disk replay steps.'],
    run_completed:['The run reaches its completion marker','The producing run reports its sustained convergence assertions passed. This is not a claim about production-network liveness.']
  };
  function story(){
    const e=state.latest,d=e.data,desc=descriptions[e.event];text('event-kind',e.event.replaceAll('_',' ').toUpperCase());text('event-title',desc[0]);text('event-description',state.provenance==='snapshots'?'Outcome reconstructed from an existing result snapshot. DAG, message exchange, and execution timing were not recorded.':desc[1]);
    C('event-facts').replaceChildren();
    const facts=[];if(d.observer)facts.push(['Observer',d.observer]);if(d.peer)facts.push(['Peer',d.peer]);if(d.count!=null)facts.push(['Ratings',d.count]);if(d.complete!=null)facts.push(['Complete',d.complete?'Yes':'No']);if(d.checkpoint)facts.push(['Checkpoint',short(d.checkpoint)]);
    if(d.excluded?.length)facts.push(['Excluded',d.excluded.join(', ')]);
    if(e.event==='round_completed'){facts.push(['Active total',total(d.weights)],['Score delta',d.max_delta??'Not recorded'],['Stable rounds',d.stable_rounds??'Not recorded']);}
    facts.forEach(([k,v])=>{const n=add(C('event-facts'),'div','');add(n,'span','',k);add(n,'strong','',String(v));});
  }
  function proof(){
    const target=C('proof');target.replaceChildren();
    const active=state.latest.event==='equivocation_verified'||state.latest.event==='penalty_verified'||state.round>=32;
    target.hidden=!active||(!state.proofs.length&&!state.penalties.length);if(target.hidden)return;
    add(target,'h3','','Equivocation evidence & penalty verification');
    state.proofs.forEach(p=>{const group=add(target,'div','cine-proof-pair');add(group,'strong','',(p.validator||'Validator')+' · evidence round '+p.evidence_round+' · checkpoint '+p.checkpoint_round);(p.blocks||[]).forEach((b,i)=>{const block=add(group,'div','cine-proof-block');add(block,'span','','Conflicting block '+(i+1));add(block,'code','',b.id);add(block,'small','',(b.parents?.length||0)+' recorded parents');});});
    const latest=new Map();state.penalties.forEach(p=>latest.set(p.validator,p));
    latest.forEach(p=>add(target,'p','',p.validator+': offending weight '+number(p.offending_weight)+' / '+number(p.total_weight)+' total ('+(p.offending_weight/p.total_weight*100).toFixed(2)+'%). Penalty '+(p.penalty/p.scale*100).toFixed(1)+'%; '+number(p.prior)+' prior → '+number(p.retained)+' retained; voting weight 0 after exclusion.'));
  }
  function ledger(){
    C('ledger').replaceChildren();const latest=state.history.at(-1),previous=state.history.at(-2);
    names.forEach((key,i)=>{const card=add(C('ledger'),'article',state.excluded.includes(key)?'cine-excluded':'');add(card,'span','cine-validator',keyName(key));add(card,'strong','',number(state.weights[key]));const share=total(state.weights)?state.weights[key]/total(state.weights)*100:0;const track=add(card,'div','cine-weight');const bar=add(track,'div','');bar.style.width=share+'%';bar.style.background=colors[i];add(card,'p','',share.toFixed(2)+'% voting share');add(card,'small','',state.excluded.includes(key)?'Excluded · retained '+number(state.retained[key]):'Active · retained '+number(state.retained[key]??0));
      if(previous&&latest){const before=previous.weights[key],after=latest.weights[key];add(card,'small','',number(before)+' → '+number(after));if(state.excluded.includes(key)&&!previous.entries?.find(x=>x.validator===key)?.excluded)add(card,'small','cine-penalty','New exclusion · '+number(state.retained[key])+' retained from '+number(before)+' prior active');}
    });
  }
  function chart(){
    const svg=C('chart');svg.replaceChildren();const history=state.history;const max=history.reduce((m,r)=>Math.max(m,...Object.values(r.weights)),1000);const planned=events[0]?.data.planned_rounds||128;
    for(const v of [0,max/2,max]){const y=145-v/max*120;svg.append(svgEl('path',{d:`M40 ${y} H660`,stroke:'#2a493f'}));svg.append(svgEl('text',{x:32,y:y+4,'text-anchor':'end',class:'cine-tiny'},Math.round(v)));}
    names.forEach((key,n)=>{if(!history.length)return;const path=history.map((r,i)=>(i?'L':'M')+(40+r.round/Math.max(1,planned)*620)+' '+(145-r.weights[key]/max*120)).join(' ');svg.append(svgEl('path',{d:path,fill:'none',stroke:colors[n],'stroke-width':2.5,'stroke-dasharray':n%2?'5 3':'none'}));svg.append(svgEl('text',{x:45+n*145,y:172,class:'cine-tiny',fill:colors[n]},keyName(key)));});
    const row=history.at(-1);text('convergence',row?'Round '+row.round+' · score Δ '+number(row.max_delta)+' · share Δ '+number(row.share_delta_ppm)+' ppm · stable '+number(row.stable_rounds)+' / 8 rounds.':'Waiting for the first completed round.');
    C('log').replaceChildren();events.slice(Math.max(0,position-5),position+1).reverse().forEach(e=>{const li=add(C('log'),'li','');add(li,'span','','#'+e.seq+' · R'+e.round);add(li,'strong','',e.event.replaceAll('_',' '));});
  }
  function draw(){
    const ready=position>=0&&state?.latest;C('empty').hidden=Boolean(ready);C('content').hidden=!ready;
    ['play','prev','next','timeline','checkpoint','latest','download'].forEach(id=>C(id).disabled=!events.length);
    C('slash').disabled=!events.some(e=>e.event==='round_completed'&&e.data.events?.includes('slash'));
    C('timeline').max=Math.max(0,events.length-1);C('timeline').value=Math.max(0,position);text('timeline-label',events.length?(position+1)+' / '+events.length:'—');
    text('status',connected?'Live connection':ready?(state.finished?'Completion recorded':'Recorded replay'):'Awaiting event trace');
    if(!ready)return;
    text('order-note',state.provenance==='snapshots'?'Outcome reports only · exchanges not traced':'Actual harness order · illustrative travel time');
    const e=state.latest;const ms=e.elapsed_ms; text('clock',state.provenance==='snapshots'?'ILLUSTRATIVE TIME':Math.floor(ms/60000).toString().padStart(2,'0')+':'+Math.floor(ms/1000%60).toString().padStart(2,'0')+'.'+(ms%1000).toString().padStart(3,'0'));
    text('run',labels[e.scenario].toUpperCase()+' · '+(state.provenance==='snapshots'?'OUTCOMES ONLY':'CAPTURED EVENTS'));text('round','ROUND '+state.round);text('count',events.length.toLocaleString()+' EVENTS');
    dag();network();story();proof();ledger();chart();
  }
  C('file').addEventListener('change',async event=>{try{const file=event.target.files[0];if(!file)return;if(file.size>64*1024*1024)throw Error('Trace files are limited to 64 MB.');const raw=await file.text();if(raw.trimStart().startsWith('{')&&!raw.includes('\n{"')){try{const parsed=JSON.parse(raw);if(parsed.kind==='por_convergence_trace'){fromSnapshots(parseTrace(parsed,file.name));return;}}catch(error){if(error.message.includes('Invalid'))throw error;}}const lines=raw.trim().split(/\r?\n/);load(lines.map(line=>JSON.parse(line)));}catch(error){msg('Could not open trace: '+error.message);}finally{event.target.value='';}});
  C('snapshots').onclick=()=>{try{fromSnapshots(current());}catch(error){msg(error.message);}};
  C('connect').onclick=connect;C('scenario').onchange=()=>{if(connected){disconnect();connect();}};
  C('play').onclick=play;C('speed').onchange=()=>{if(playing){pause();play();}};
  const manual=next=>{pause();C('tail').checked=false;seek(next);};
  C('next').onclick=()=>manual(position+1);C('prev').onclick=()=>manual(position-1);C('timeline').oninput=e=>manual(Number(e.target.value));C('latest').onclick=()=>manual(events.length-1);
  C('checkpoint').onclick=()=>{const n=events.findIndex((e,i)=>i>position&&e.event===(state.provenance==='snapshots'?'round_completed':'checkpoint_started'));if(n>=0)manual(n);else msg('No later checkpoint-start event has arrived.');};
  C('slash').onclick=()=>{const n=events.findIndex(e=>e.event==='round_completed'&&e.data.events?.includes('slash'));if(n>=0)manual(n);};
  C('window').oninput=()=>{C('follow').checked=false;if(state?.latest)dag();};C('zoom').onchange=()=>{if(state?.latest)dag();};C('follow').onchange=()=>{if(state?.latest)dag();};C('tail').onchange=()=>{if(C('tail').checked)seek(events.length-1);};
  C('download').onclick=()=>{const blob=new Blob([events.map(e=>JSON.stringify(e)).join('\n')+'\n'],{type:'application/x-ndjson'});const url=URL.createObjectURL(blob),a=el('a');a.href=url;a.download=(events[0]?.scenario||'por')+'.events.jsonl';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);};
  document.addEventListener('visibilitychange',()=>{if(document.hidden)pause();});
  document.getElementById('explorer-tab').addEventListener('click',pause);
  document.querySelector('.legacy-walkthrough').addEventListener('toggle',e=>{if(e.target.open)pause();});
  document.addEventListener('keydown',e=>{
    if(document.getElementById('animation-view').hidden||document.getElementById('import-dialog').open||/^(INPUT|SELECT|BUTTON|TEXTAREA|A|SUMMARY)$/.test(e.target.tagName)||e.ctrlKey||e.metaKey||e.altKey)return;
    if(e.key===' '){e.preventDefault();play();}else if(e.key==='ArrowRight'){e.preventDefault();manual(position+1);}else if(e.key==='ArrowLeft'){e.preventDefault();manual(position-1);}
  });
  window.addEventListener('beforeunload',disconnect);
  state=emptyState();draw();
})();