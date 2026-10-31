/* Educational stage timing; only checkpoint snapshots are recorded evidence. */
(() => {
  const stages = ['Checkpoint', 'Ratings', 'Agreement', 'Reputation', 'Slashing', 'Convergence'];
  let stage = 0, clock = null, identity = '';
  const put = (id, text) => { $(id).textContent = text; };
  function pause() { clearInterval(clock); clock = null; put('replay-play', 'Play walkthrough'); }
  window.pausePorReplay = pause;
  function step(direction) {
    if (!rows().length) return;
    stage += direction;
    if (stage > 5) { if (index === rows().length - 1) { stage = 5; pause(); } else { stage = 0; index++; } }
    if (stage < 0) { if (index > 0) { index--; stage = 5; } else stage = 0; }
    identity = selected + ':' + index; render();
  }
  function toggle() {
    if (clock) return pause();
    if (!rows().length) return;
    stop();
    if (index === rows().length - 1 && stage === 5) { index = 0; stage = 0; }
    identity = selected + ':' + index; render();
    put('replay-play', 'Pause walkthrough');
    clock = setInterval(() => step(1), Number($('replay-speed').value));
  }
  function tab(animation) {
    pause(); stop();
    $('animation-view').hidden = !animation; $('explorer-view').hidden = animation;
    ['explorer-tab', 'animation-tab'].forEach((id, n) => {
      $(id).setAttribute('aria-selected', String(Boolean(n) === animation));
      $(id).tabIndex = Boolean(n) === animation ? 0 : -1;
    });
    render();
  }
  $('explorer-tab').onclick = () => tab(false);
  $('animation-tab').onclick = () => tab(true);
  document.querySelector('.replay-tabs').addEventListener('keydown', e => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    e.preventDefault(); const animation = e.key === 'End' || (e.key !== 'Home' && $('animation-view').hidden);
    tab(animation); $(animation ? 'animation-tab' : 'explorer-tab').focus();
  });
  $('replay-play').onclick = toggle;
  $('replay-next').onclick = () => { pause(); step(1); };
  $('replay-back').onclick = () => { pause(); step(-1); };
  $('replay-speed').onchange = () => { if (clock) { pause(); toggle(); } };
  $('replay-slider').oninput = e => { pause(); stage = 0; selectRound(Number(e.target.value)); };
  $('replay-fault').onclick = () => { pause(); index = firstFault(); stage = 4; identity = selected + ':' + index; render(); };
  $('replay-end').onclick = () => { pause(); index = rows().length - 1; stage = 5; identity = selected + ':' + index; render(); };
  document.addEventListener('visibilitychange', () => { if (document.hidden) pause(); });
  const make = (parent, tag, cls, text) => { const n = el(tag, cls, text); parent.append(n); return n; };
  function drawDag(row) {
    const svg = $('replay-dag'); svg.replaceChildren(); svg.classList.toggle('flowing', Boolean(row) && stage === 1);
    const active = stage === 0 ? 2 : 5;
    for (let n = 0; n < 6; n++) {
      const x = 48 + n * 112;
      if (n) svg.append(svgEl('path', {d:`M${x-72} 85 H${x-30}`, class:'dag-edge'}));
      svg.append(svgEl('rect', {x:x-27,y:57,width:54,height:56,rx:10,class:n<=active?'dag-block reached':'dag-block'}));
      svg.append(svgEl('text', {x,y:90,'text-anchor':'middle',class:'dag-label'}, 'B' + (n + 1)));
      svg.append(svgEl('text', {x,y:139,'text-anchor':'middle',class:'dag-sub'}, 'illustration'));
    }
    svg.append(svgEl('text', {x:340,y:185,'text-anchor':'middle',class:'dag-sub'}, row ? 'Checkpoint ' + row.round + ' · parent-linked blocks → rating evidence' : 'Import a scenario to begin'));
  }
  function trend(data, names) {
    const svg = $('replay-trend'); svg.replaceChildren(); $('replay-legend').replaceChildren();
    const ymax = data.reduce((maximum, r) => Object.values(r.weights).reduce((m, v) => Math.max(m, v), maximum), 1);
    const x = i => 35 + i / Math.max(1, data.length-1) * 625;
    const y = v => 135 - v/ymax*115;
    [0, ymax/2, ymax].forEach(v => { svg.append(svgEl('path',{d:`M35 ${y(v)} H660`,class:'replay-gridline'})); svg.append(svgEl('text',{x:30,y:y(v)+4,'text-anchor':'end',class:'dag-sub'},String(Math.round(v)))); });
    names.forEach((key, n) => {
      const color=colors[n%colors.length]; const shown=data.slice(0,index+1);
      if (shown.length) {
        svg.append(svgEl('path',{d:shown.map((r,i)=>(i?'L':'M')+x(i)+' '+y(r.weights[key])).join(' '),fill:'none',stroke:color,'stroke-width':2.5,'stroke-dasharray':n%2?'5 3':'none'}));
        svg.append(svgEl('circle',{cx:x(index),cy:y(shown.at(-1).weights[key]),r:4,fill:color}));
      }
      const label=el('span',''); label.append(swatch(color),document.createTextNode(keyName(key))); $('replay-legend').append(label);
    });
    svg.append(svgEl('text',{x:35,y:160,class:'dag-sub'},data.length?'Round '+data[0].round:''));
    svg.append(svgEl('text',{x:660,y:160,'text-anchor':'end',class:'dag-sub'},data.length?'Round '+data.at(-1).round:''));
  }
  window.renderPorReplay = function () {
    if (!$('animation-view')) return;
    const data=rows(), row=data[index], prev=data[index-1], run=current();
    const nextIdentity=selected+':'+index;
    if(identity!==nextIdentity) { stage=0; identity=nextIdentity; }
    document.querySelector('.replay-shell').classList.toggle('replay-empty', !row);
    const names=row?Object.keys(row.weights).sort((a,b)=>Number(a.split('-')[1])-Number(b.split('-')[1])):[];
    const faults=row?names.filter(k=>excluded(row,k)&&(!prev||!excluded(prev,k))):[];
    const events=row?eventsAt(row,index):[];
    ['replay-play','replay-next','replay-back','replay-speed','replay-slider','replay-end'].forEach(id=>$(id).disabled=!row);
    $('replay-fault').disabled=firstFault()<0;
    $('replay-slider').max=Math.max(0,data.length-1); $('replay-slider').value=index;
    $('replay-slider').setAttribute('aria-valuetext',row?'Round '+row.round:'No recorded rounds');
    put('replay-round',row?row.round:'—'); put('replay-position',row?labels[selected]+' · '+(index+1)+' / '+data.length:'No results loaded');
    $('replay-stages').replaceChildren();
    stages.forEach((name,n)=>{const b=make($('replay-stages'),'button',n===stage?'active':n<stage?'complete':'',String(n+1).padStart(2,'0')+' '+name); b.disabled=!row; b.setAttribute('aria-pressed',String(n===stage)); b.onclick=()=>{pause();stage=n;window.renderPorReplay();};});
    const explanations = [
      ['Process illustration','Establish the checkpoint window.','The test consumes a fixed, ordered block fixture. A checkpoint binds the evidence used for this round. The chain below illustrates that relationship; it does not reconstruct actual block identities or a live local DAG.'],
      ['Process illustration','Collect evidence-backed ratings.','Eligible observers sign ratings for admitted block-production interactions. Missing required peer ratings keep a checkpoint open. A peer with zero required ratings is already complete. The file does not record individual signatures or arrival order.'],
      ['Recorded outcome','Compare the observer outcomes.',row?.observer_agreement===true?'The trace reports observer agreement. The producing test checks matching reputation checkpoint hashes and full reputation states across four independent stores. This is checkpoint agreement, not a live network quorum animation.':'This file does not record an explicit per-round observer-agreement result. No agreement is inferred from identical-looking illustrations.'],
      ['Recorded outcome','Read the updated reputation.', 'The ledger shows the recorded end-of-round state, including any penalty. Differences are against the previous exported snapshot. Intermediate pre-penalty scores are not exported, so this stage does not invent them.'],
      ['Recorded outcome',faults.length?'Apply the fault outcome.':'Inspect exclusion and retained reputation.',faults.length?'Newly excluded validators lose all voting weight. Retained reputation is tracked separately; it grants no voting power. The amounts below are derived from the recorded snapshots, not an animation of signature verification.':'No new exclusion is visible at this checkpoint. Previously excluded identities remain at zero voting weight; a restart does not restore their eligibility.'],
      ['Recorded outcome','Measure whether the system settles.','Convergence requires score changes and voting-share changes to remain within their bounds, with unchanged ranking over the required consecutive window. Flat curves alone do not prove new blocks are finalizing.']
    ];
    const story=explanations[stage]; put('replay-evidence',story[0]); put('replay-title',row?story[1]:'Choose a result to replay.'); put('replay-description',row?story[2]:'Import the scenario JSON using Import results, or select the guided example. No synthetic trajectory is substituted for missing results.');
    drawDag(row);
    put('replay-dag-note',run?.example?'This historical fixture has no exported DAG trace. Block shapes are illustrative.':'The current convergence test fixture uses a single-producer chain (V1), not balanced production by four validators. Only a representative chain is illustrated.');
    $('replay-observers').replaceChildren();
    const count=Number.isInteger(run?.observerCount) && run.observerCount > 0 ? run.observerCount : (run?.example ? 4 : 0);
    if (!count) make($('replay-observers'),'p','replay-caption','Observer count not recorded.');
    for(let n=0;n<Math.min(count,16);n++) {
      const card=make($('replay-observers'),'div',stage===2?'observer-lit':'');
      make(card,'strong','', 'Observer '+(n+1)); make(card,'span','',!row?'No snapshot':row.observer_agreement===true?'Agreement reported':'Not recorded');
    }
    put('replay-state-note','End-of-round snapshot · '+(row?'round '+row.round:'no data')+' · bars show voting share');
    $('replay-balances').replaceChildren();
    names.forEach((key,n)=>{
      const entry=knownEntry(row,key), before=prev?.weights[key], value=row.weights[key], share=total(row.weights)?value/total(row.weights)*100:0;
      const card=make($('replay-balances'),'div','replay-validator');
      const title=make(card,'div','section-label'); make(title,'strong','',keyName(key)); make(title,'span','',excluded(row,key)?'Excluded':value>0?'Active':'Zero weight');
      const track=make(card,'div','replay-weight-track');const bar=make(track,'div','');bar.style.width=share+'%';bar.style.background=colors[n%colors.length];
      make(card,'p','', (before==null?'Previous: not exported':number(before))+' → '+number(value)+' active · '+share.toFixed(2)+'% share');
      make(card,'small','', 'Retained: '+(entry?number(entry.retained_reputation):'not recorded')+' · Change: '+(before==null?'not available':(value-before>0?'+':'')+number(value-before)));
    });
    $('replay-penalty').replaceChildren();
    make($('replay-penalty'),'strong','',faults.length?'New exclusions at this checkpoint':'Fault / recovery record');
    if(faults.length) faults.forEach(key=>{
      const entry=knownEntry(row,key), before=prev?.weights[key];
      make($('replay-penalty'),'p','',keyName(key)+': '+(before==null?'Prior balance not exported':number(before)+' prior active')+'; '+number(entry?.retained_reputation)+' retained; 0 voting weight.'+(before>0&&entry?' Retained balance is '+(entry.retained_reputation/before*100).toFixed(1)+'% of prior active reputation.':''));
    });
    else make($('replay-penalty'),'p','',events.includes('restart')?'Restart marker: this is the first exported checkpoint after recovery. Recovery internals are not traced.':'No new fault or recovery event recorded here.');
    trend(data,names);
    put('replay-stability',row?'Score Δ '+number(row.max_delta)+' / '+limits().score_delta+' · Share Δ '+number(row.share_delta_ppm)+' / '+limits().share_delta_ppm+' ppm · Ranking '+(row.rank_stable==null?'not recorded':row.rank_stable?'unchanged':'changed')+' · Stable window '+number(row.stable_rounds)+' / '+limits().stable_rounds+'. The convergence test counts this window after round 32.':'No convergence measurements loaded.');
    $('replay-record').replaceChildren();
    [['Round',row?.round??'—'],['Active total',row?number(total(row.weights)):'—'],['Events',events.join(', ')||'None recorded'],['Checkpoint',row?.checkpoint||'Not recorded'],['Run status',run?.status?run.status+' (reported by file)':'Not reported']].forEach(([k,v])=>{make($('replay-record'),'dt','',k);make($('replay-record'),'dd','',v);});
  };
  window.renderPorReplay();
  if (location.hash === '#animation') tab(true);
})();