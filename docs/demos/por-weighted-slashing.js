/* Local-only PoR explorer. Imported files are data, never executable content. */
"use strict";
const $ = id => document.getElementById(id);
const el = (tag, cls, value) => {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (value !== undefined) node.textContent = String(value);
  return node;
};
const svgEl = (tag, attrs = {}, text) => {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, String(value)));
  if (text !== undefined) node.textContent = String(text);
  return node;
};
const number = value => value == null ? "—" : value.toLocaleString("en-US");
const short = value => value ? value.slice(0, 10) + "…" + value.slice(-7) : "Not recorded";
const labels = {honest:"Honest baseline", unequal:"Unequal start", isolated:"Isolated fault", correlated:"Correlated fault", example:"Guided example"};
const colors = ["#2c6c52", "#be754c", "#6c8da4", "#b3a05d", "#8a79a5", "#628a7d"];
const runs = new Map();
let selected = "example", index = 0, measure = "reputation", timer = null;
let fixture = null, currentHash = null, latestLive = null, liveReachable = false, lastLiveProgress = null;
const current = () => runs.get(selected);
const rows = () => current()?.rows || [];
const keyName = key => key.replace(/^validator-/, "Validator ");
const validNumber = value => Number.isSafeInteger(value) && value >= 0;
const assert = (condition, message) => { if (!condition) throw Error(message); };
const total = weights => Object.values(weights).reduce((a, b) => a + b, 0);
const knownEntry = (row, key) => row.entries?.find(entry => entry.validator === key);
const excluded = (row, key) => knownEntry(row, key)?.excluded === true;
const limits = () => current()?.thresholds || {score_delta:1, share_delta_ppm:1000, stable_rounds:8};
function swatch(color) { const node=el("span","legend-swatch"); node.style.background=color; return node; }
function notify(message) { $("notice").textContent=message; $("notice").hidden=!message; }
function stop() {
  clearInterval(timer); timer=null;
  $("play").textContent="▶"; $("play").setAttribute("aria-label","Play recorded rounds");
}
function play() {
  if (timer) return stop();
  if (rows().length < 2) return;
  if (index >= rows().length-1) index=0;
  $("play").textContent="Ⅱ"; $("play").setAttribute("aria-label","Pause playback");
  render();
  timer=setInterval(() => {
    if (index >= rows().length-1) return stop();
    index++; render();
  }, Number($("speed").value));
}
function selectScenario(name) { window.pausePorReplay?.(); stop(); selected=name; index=0; notify(""); render(); }
function selectRound(next) { index=Math.max(0,Math.min(rows().length-1,next)); render(); }

function parseTrace(data, filename) {
  const legacy=Array.isArray(data);
  assert(legacy || (data?.schema_version===1 && data.kind==="por_convergence_trace"), "Unsupported trace format.");
  const raw=legacy?data:data.rounds;
  assert(Array.isArray(raw)&&raw.length>0&&raw.length<=10000, "Expected 1–10,000 recorded rounds.");
  const scenario=legacy?raw[0]?.scenario:data.scenario;
  assert(Object.hasOwn(labels,scenario)&&scenario!=="example","Unknown scenario name.");
  let keys=null, previousRound=-1;
  const parsed=raw.map((row, offset) => {
    assert(row && validNumber(row.round) && row.round>previousRound, "Rounds must be increasing nonnegative integers.");
    previousRound=row.round;
    assert(!row.scenario || row.scenario===scenario,"Mixed scenarios in one file.");
    assert(row.weights && typeof row.weights==="object" && !Array.isArray(row.weights),"Missing weight map.");
    const names=Object.keys(row.weights).sort();
    assert(names.length>0&&names.length<=64&&names.every(key=>/^validator-[1-9][0-9]*$/.test(key)), "Expected named validator weights.");
    assert(names.every(key=>validNumber(row.weights[key]))&&Number.isSafeInteger(total(row.weights)), "Weights must be nonnegative safe integers.");
    if (keys) assert(JSON.stringify(names)===JSON.stringify(keys),"Validator membership changes inside this trace.");
    keys=names;
    for (const field of ["max_delta","share_delta_ppm","stable_rounds"])
      assert(row[field]==null||validNumber(row[field]),"Invalid "+field+" at record "+offset);
    assert(row.rank_stable==null||typeof row.rank_stable==="boolean","Invalid ranking stability.");
    assert(row.total_weight==null||row.total_weight===total(row.weights),"Total weight does not match the weight map.");
    assert(row.checkpoint==null||/^[a-f0-9]{64}$/i.test(row.checkpoint),"Invalid checkpoint hash.");
    const entries=[];
    if(row.entries!=null) {
      assert(Array.isArray(row.entries),"Invalid reputation entries.");
      const seen=new Set();
      for(const entry of row.entries) {
        assert(entry&&validNumber(entry.reputation)&&validNumber(entry.retained_reputation)&&typeof entry.excluded==="boolean","Invalid reputation entry.");
        // Old exports contain only public keys. Never guess their label by array position.
        if(entry.validator!=null) {
          assert(names.includes(entry.validator)&&!seen.has(entry.validator),"Invalid or duplicate validator entry.");
          seen.add(entry.validator);
          assert(row.weights[entry.validator]===(entry.excluded?0:entry.reputation),"Entry and projected weight disagree.");
          entries.push({...entry});
        }
      }
    }
    const events=Array.isArray(row.events)?row.events.filter(e=>["slash","restart","activation"].includes(e)):[];
    return {...row, weights:{...row.weights}, entries, events};
  });
  let thresholds={score_delta:1,share_delta_ppm:1000,stable_rounds:8};
  if(!legacy&&data.thresholds) {
    thresholds=data.thresholds;
    for(const field of ["score_delta","share_delta_ppm","stable_rounds"])
      assert(validNumber(thresholds[field])&&thresholds[field]>0,"Invalid convergence thresholds.");
  }
  const status=!legacy&&["passed","failed"].includes(data.result?.status)?data.result.status:"not reported";
  const final=parsed.at(-1);
  if(status==="passed") assert(final.stable_rounds>=thresholds.stable_rounds && total(final.weights)>0,
    "The file reports a pass but its final stability window or active weight does not meet the stated bounds.");
  return {scenario,rows:parsed,thresholds,status,filename,example:false,observerCount:legacy?null:data.observer_count};
}

function parseFixture(data) {
  assert(data?.kind==="deterministic_checkpoint_fixture"&&data.schema_version===1&&data.observers?.length===4,"Invalid guided fixture.");
  const base=data.observers[0];
  for(const key of ["round_1_checkpoint","round_2_checkpoint","round_1_weights","round_2_weights"])
    assert(data.observers.every(o=>JSON.stringify(o[key])===JSON.stringify(base[key])),"Fixture observers disagree.");
  assert(Array.isArray(data.proof_blocks)&&data.proof_blocks.length===2&&data.proof_blocks.every(hash=>/^[a-f0-9]{64}$/i.test(hash)),"Invalid fixture evidence.");
  assert(/^[a-f0-9]{64}$/i.test(data.penalties_hash)&&validNumber(base.round_1_retained),"Invalid fixture penalty.");
  assert(base.round_1_weights[data.offender]===0&&base.round_2_weights[data.offender]===0,"Fixture offender is not excluded.");
  const names=Object.keys(data.initial_weights);
  assert(names.length===4&&names.every(key=>/^validator-[1-9][0-9]*$/.test(key)),"Invalid fixture identities.");
  const result=[0,1,2].map(round=>{
    const weights=round===0?data.initial_weights:base["round_"+round+"_weights"];
    assert(names.every(key=>validNumber(weights[key])),"Invalid fixture weights.");
    return {round,weights,checkpoint:round?base["round_"+round+"_checkpoint"]:null,
      entries:names.map(validator=>({validator,reputation:weights[validator],retained_reputation:round&&validator===data.offender?base.round_1_retained:0,excluded:Boolean(round&&validator===data.offender)})),
      observer_agreement:round>0,events:round===1?["slash"]:round===2?["restart"]:[]};
  });
  return {scenario:"example",rows:result,example:true,status:"historical example",filename:"por-weighted-slashing.json",observerCount:4,evidence:data};
}
function eventsAt(row, offset) {
  const found=new Set(row.events||[]);
  const prev=rows()[offset-1];
  if(row.entries?.some(entry=>entry.excluded&&(!prev||!excluded(prev,entry.validator)))) found.add("slash");
  return [...found];
}
function firstFault() { return rows().findIndex((row,i)=>eventsAt(row,i).includes("slash")); }
function render() {
  const run=current(), data=rows(), row=data[index], prev=data[index-1];
  document.querySelectorAll("[data-scenario]").forEach(button=>{
    const loaded=runs.get(button.dataset.scenario);
    button.classList.toggle("active",selected===button.dataset.scenario);
    button.classList.toggle("loaded",Boolean(loaded));
    button.setAttribute("aria-pressed",String(selected===button.dataset.scenario));
    button.querySelector(".scenario-state").textContent=loaded?loaded.rows.length+" rounds":"No results";
  });
  $("source-title").textContent=run?(run.example?"Guided example · saved two-round fixture":labels[selected]+" · imported result") : labels[selected]+" · no results loaded";
  $("source-description").textContent=run?(run.example?"Historical recorded checkpoints, not results from the new convergence tests.":run.filename+" · Result reported by file: "+run.status+". The browser does not rerun assertions."):"Import this scenario’s JSON to inspect its actual trajectory. No simulated values are filled in.";
  $("workspace-status").replaceChildren(document.createTextNode(run?(run.example?"Example loaded":"Result loaded"):"Awaiting results"),el("small","",run?(run.example?"Import a run to inspect new results.":data.length+" recorded rounds · "+run.status):"Choose an exported JSON file."));
  const enabled=Boolean(row);
  $("chart-empty").hidden=enabled;
  ["play","round-slider","jump-start","jump-end","speed"].forEach(id=>$(id).disabled=!enabled||data.length<2);
  $("jump-fault").disabled=firstFault()<0;
  $("round-slider").max=Math.max(0,data.length-1); $("round-slider").value=index;
  $("round-slider").setAttribute("aria-valuetext",row?"Round "+row.round:"No rounds loaded");
  $("timeline-start").textContent=data.length?"Round "+data[0].round:"—";
  $("timeline-end").textContent=data.length?"Round "+data.at(-1).round:"—";
  $("timeline-current").textContent=row?"Round "+row.round:"No result";
  $("metric-round").textContent=row?String(row.round).padStart(2,"0"):"—";
  $("metric-round-note").textContent=row?data.length+" recorded snapshots":"Finalized PoR checkpoint";
  $("metric-total").textContent=row?number(total(row.weights)):"—";
  const names=row?Object.keys(row.weights).sort((a,b)=>Number(a.split("-")[1])-Number(b.split("-")[1])):[];
  $("metric-active").textContent=row?names.filter(key=>row.weights[key]>0).length+" / "+names.length:"—";
  $("metric-active-note").textContent="Validators with positive voting weight";
  $("metric-stability").textContent=row?.stable_rounds!=null?row.stable_rounds+" / "+limits().stable_rounds:"—";
  $("metric-stability-note").textContent=row?.stable_rounds!=null?"Recorded consecutive rounds within bounds":"Not measured in this "+(run?.example?"example":"view");
  drawChart(run,data);
  drawInspector(run,row,prev,names);
  drawLedger(row,prev,names);
  drawChecks(run,row);
  window.renderPorReplay?.();
  $("event-track").replaceChildren();
  let lastMarker=-100;
  data.forEach((sample,i)=>{
    const kinds=eventsAt(sample,i);
    if(!kinds.length)return;
    const position=data.length>1?i/(data.length-1)*100:0;
    if(position-lastMarker<13 && !kinds.includes("slash"))return;
    lastMarker=position;
    const marker=el("span","",kinds.includes("slash")?"Slash":"Restart");
    marker.style.left=Math.min(92,Math.max(8,position))+"%";
    marker.title=kinds.join(", ")+" · round "+sample.round;
    $("event-track").append(marker);
  });
}
function drawChart(run,data) {
  const chart=$("trajectory"); chart.replaceChildren(); $("legend").replaceChildren();
  document.querySelectorAll("[data-measure]").forEach(button=>button.setAttribute("aria-pressed",String(button.dataset.measure===measure)));
  const description={
    reputation:["Active reputation","Absolute scores. A drop in one validator’s score is different from a change in everyone’s voting share."],
    share:["Share of voting power","Each active score divided by the total. Shares sum to 100%; retained capital has no voting power."],
    delta:["Distance from stability","Score and share changes divided by their acceptance bounds. Both should stay at or below 1×; rankings must also settle."]
  };
  $("chart-title").textContent=description[measure][0]; $("chart-description").textContent=description[measure][1];
  const width=Math.max(280,chart.clientWidth||820),height=window.innerWidth<720?240:310;
  const left=50,right=width-22,top=26,bottom=height-40;
  chart.setAttribute("viewBox","0 0 "+width+" "+height);
  chart.dataset.left=left;chart.dataset.right=right;chart.dataset.width=width;
  let series=[], ymax=1;
  if(data.length) {
    if(measure==="delta") {
      series=[{name:"Score change / bound",color:colors[0],values:data.map(r=>r.max_delta==null?null:r.max_delta/limits().score_delta)},
        {name:"Share change / bound",color:colors[1],values:data.map(r=>r.share_delta_ppm==null?null:r.share_delta_ppm/limits().share_delta_ppm)}];
      ymax=series.reduce((maximum,s)=>s.values.reduce((m,v)=>v==null?m:Math.max(m,Math.log10(1+v)),maximum),1);
    } else series=Object.keys(data[0].weights).sort((a,b)=>Number(a.split("-")[1])-Number(b.split("-")[1])).map((key,i)=>({
      name:keyName(key),color:colors[i%colors.length],values:data.map(r=>measure==="share"?(total(r.weights)?r.weights[key]/total(r.weights)*100:0):r.weights[key])
    }));
    if(measure==="share")ymax=100;
    if(measure==="reputation")ymax=series.reduce((maximum,s)=>s.values.reduce((m,v)=>Math.max(m,v),maximum),1)*1.12;
  }
  series.forEach(s=>{const item=el("span","legend-item");item.append(swatch(s.color),document.createTextNode(s.name));$("legend").append(item);});
  const first=data[0]?.round??0,last=data.at(-1)?.round??128;
  const x=round=>left+(round-first)/Math.max(1,last-first)*(right-left);
  const y=value=>bottom-(measure==="delta"?Math.log10(1+value):value)/ymax*(bottom-top);
  const add=node=>chart.append(node);
  add(svgEl("title",{},description[measure][0]+" across recorded checkpoints"));
  add(svgEl("desc",{},data.length?"Use the round slider or click the chart to inspect a recorded sample.":"No scenario result is loaded."));
  for(let i=0;i<=4;i++) {
    const value=ymax*i/4, py=bottom-i/4*(bottom-top);
    add(svgEl("line",{x1:left,x2:right,y1:py,y2:py,stroke:"#e8eee1","stroke-dasharray":i===0?"":"3 5"}));
    const label=measure==="delta"?number(Math.round(10**value-1))+"×":measure==="share"?Math.round(value)+"%":number(Math.round(value));
    add(svgEl("text",{x:left-12,y:py+4,"text-anchor":"end",fill:"#7f926e","font-size":10,"font-family":"monospace"},label));
  }
  const ticks=[...new Set(Array.from({length:Math.min(5,Math.max(2,data.length))},(_,i)=>Math.round(first+(last-first)*i/(Math.min(5,Math.max(2,data.length))-1))))];
  ticks.forEach(round=>add(svgEl("text",{x:x(round),y:bottom+25,"text-anchor":"middle",fill:"#7f926e","font-size":10,"font-family":"monospace"},round)));
  const fault=firstFault();
  if(fault>=0) {
    const fx=x(data[fault].round);
    add(svgEl("line",{x1:fx,x2:fx,y1:top,y2:bottom,stroke:"#c59374","stroke-dasharray":"4 5"}));
    add(svgEl("text",{x:Math.min(right-48,fx+7),y:top-8,fill:"#a76b43","font-size":10},"SLASH"));
  }
  if(measure==="delta"&&data.some(row=>row.max_delta!=null)) {
    const py=y(1);
    add(svgEl("line",{x1:left,x2:right,y1:py,y2:py,stroke:"#879f6d","stroke-dasharray":"6 4"}));
    add(svgEl("text",{x:right,y:py-6,"text-anchor":"end",fill:"#657e4e","font-size":9},"ACCEPTANCE BOUND · 1×"));
  }
  series.forEach((s,si)=>{
    let path="",started=false;
    s.values.forEach((value,i)=>{
      if(value==null){started=false;return;}
      path+=(started?" L":" M")+x(data[i].round).toFixed(2)+","+y(value).toFixed(2);
      started=true;
    });
    if(path)add(svgEl("path",{d:path,fill:"none",stroke:s.color,"stroke-width":2.4,"stroke-linejoin":"round","stroke-linecap":"round","stroke-dasharray":si===2?"7 3":si===3?"2 4":""}));
    if(data.length<=8)s.values.forEach((v,i)=>{if(v!=null)add(svgEl("circle",{cx:x(data[i].round),cy:y(v),r:3,fill:"white",stroke:s.color,"stroke-width":1.5}));});
  });
  if(data[index]) {
    const px=x(data[index].round);
    add(svgEl("line",{x1:px,x2:px,y1:top+4,y2:bottom,stroke:"#304e39","stroke-width":1,"stroke-dasharray":"2 4"}));
    series.forEach(s=>{const v=s.values[index];if(v!=null)add(svgEl("circle",{cx:px,cy:y(v),r:4.5,fill:"white",stroke:s.color,"stroke-width":2}));});
  }
  $("chart-range").textContent=data.length?"Rounds "+first+"—"+last:"No samples";
  $("chart-footnote").textContent=measure==="delta"?(data.some(r=>r.max_delta!=null)?"Logarithmic axis: 0 is no change; 1× is the acceptance bound.":"This example does not record convergence metrics."):"Lines connect recorded samples. They do not represent unrecorded intermediate states.";
}

function drawInspector(run,row,prev,names) {
  const event=row?eventsAt(row,index):[];
  $("inspector-round").textContent=row?String(row.round).padStart(2,"0"):"—";
  let title="Ready when you are.",label="No recorded data",description="Import a result file to inspect this scenario. The chart and ledger will show its recorded checkpoints.";
  let note="Voting share is active reputation divided by the total active reputation.";
  if(row) {
    const newlyExcluded=names.filter(key=>excluded(row,key)&&(!prev||!excluded(prev,key)));
    if(event.includes("slash")) {
      label="Slashing checkpoint";title="The key loses its vote.";
      const retained=names.filter(key=>excluded(row,key)).reduce((sum,key)=>sum+(knownEntry(row,key)?.retained_reputation||0),0);
      description=(newlyExcluded.length?newlyExcluded.map(keyName).join(" and ")+" "+(newlyExcluded.length===1?"is":"are")+" excluded. ":"A slash is recorded here. ")+
        (retained?number(retained)+" reputation is retained separately; it contributes no voting weight.":"The recorded excluded keys have no retained capital.");
      const priorCapital=prev?newlyExcluded.reduce((sum,key)=>sum+prev.weights[key],0):0;
      if(priorCapital>0)description+=" "+((1-retained/priorCapital)*100).toFixed(0)+"% of their pre-slash reputation is removed.";
      note="Other validators can gain voting share without earning more reputation: the total active weight has become smaller.";
    } else if(event.includes("restart")) {
      label="Recovery checkpoint";title="State survives the restart.";
      description="This record follows a restart. Check the same excluded identities, retained balances, and checkpoint history. The imported trace is the evidence; playback does not restart a node.";
      note="Recovery needs two checks: preserving the old finalized prefix and then finalizing new blocks.";
    } else if(row.stable_rounds>=limits().stable_rounds) {
      label="Within recorded bounds";title="A sustained settling period.";
      description="The file records "+row.stable_rounds+" consecutive rounds inside the score and voting-share bounds, with an unchanged ranking.";
      note="Numerical stability is one property. It does not by itself prove agreement, resistance to collusion, or consensus liveness.";
    } else if(row.round===0) {
      label="Starting state";title="A shared starting point.";
      description="Each validator starts with "+number(row.weights[names[0]])+" active reputation in this saved example. Equal scores give each key an equal share of voting power.";
      note="Use the slider to move to round 1, where the verified isolated slash is applied.";
    } else {
      label=row.rank_stable===false?"Ranking changed":"Checkpoint update";
      title=run.example?"The exclusion persists.":"Ratings become weight.";
      description=run.example?"The next checkpoint keeps Validator 2 excluded and its retained balance separate. The other scores continue through the reputation transition.":
        "At this checkpoint, ratings have updated the reputation vector. Compare exact score changes in the ledger and switch to voting share to see their relative effect.";
      if(row.stable_rounds!=null)note="This round records "+number(row.max_delta)+" maximum score change and "+number(row.share_delta_ppm)+" ppm maximum voting-share change. "+(row.rank_stable?"Ranking is unchanged.":"Ranking changed.");
    }
  }
  $("event-label").textContent=label;$("inspector-title").textContent=title;$("inspector-description").textContent=description;$("inspector-note").textContent=note;
  $("distribution").replaceChildren();$("share-list").replaceChildren();
  const sum=row?total(row.weights):0;
  $("distribution-total").textContent=sum?"100%":row?"No active weight":"—";
  names.forEach((key,i)=>{
    const share=sum?row.weights[key]/sum*100:0;
    const segment=el("span");segment.style.background=colors[i%colors.length];segment.style.width=share+"%";segment.title=keyName(key)+": "+share.toFixed(2)+"%";
    $("distribution").append(segment);
    const item=el("div"),name=el("span","share-name");
    name.append(swatch(colors[i%colors.length]),document.createTextNode(keyName(key)));
    item.append(name,el("b","",share.toFixed(2)+"%"));$("share-list").append(item);
  });
  currentHash=row?.checkpoint||null;
  $("checkpoint-hash").textContent=currentHash?short(currentHash):row?.round===0?"Before first checkpoint":"Not recorded";
  $("checkpoint-hash").title=currentHash||"";
  $("copy-checkpoint").disabled=!currentHash;
  const proof=$("proof-details"); proof.replaceChildren();
  proof.append(el("summary","",run?.example?"Inspect the evidence":"About this trace"));
  if(run?.evidence) {
    proof.append(el("p","","Two distinct, signed branches from one key were verified as incomparable. Their authorization is included in a "+run.evidence.source_prefix_len+"-block finalized source window."));
    for(const [i,hash] of run.evidence.proof_blocks.entries()) {
      proof.append(el("p","","Signed branch "+(i+1)),el("code","",hash));
    }
    proof.append(el("p","","Penalty commitment"),el("code","",run.evidence.penalties_hash));
  } else {
    proof.append(el("p","","This file records reputation outcomes and checkpoint identity. Raw equivocation proofs and finalized source blocks are not embedded in the trajectory. Inspect retained host evidence for an independent audit."));
  }
}
function drawLedger(row,prev,names) {
  const body=$("validator-rows");body.replaceChildren();
  if(!row){const tr=el("tr"),td=el("td","","No recorded validator state.");td.colSpan=6;tr.append(td);body.append(tr);return;}
  const sum=total(row.weights);
  names.forEach((key,i)=>{
    const tr=el("tr"),name=el("td"),nameWrap=el("span","validator-name");
    nameWrap.append(swatch(colors[i%colors.length]),document.createTextNode(keyName(key)));name.append(nameWrap);
    const change=prev?row.weights[key]-prev.weights[key]:null,entry=knownEntry(row,key);
    const status=entry?.excluded?"Ejected":row.weights[key]>0?"Weighted":"Zero weight";
    const state=el("td");state.append(el("span","state-pill"+(entry?.excluded?" ejected":!row.weights[key]?" unknown":""),status));
    tr.append(name,el("td","",number(row.weights[key])),
      el("td",change<0?"value-negative":change>0?"value-positive":"",change==null?"—":change>0?"+"+number(change):number(change)),
      el("td","",number(entry?.retained_reputation)),el("td","",sum?(row.weights[key]/sum*100).toFixed(2)+"%":"—"),state);
    body.append(tr);
  });
}
function drawChecks(run,row) {
  $("checks").replaceChildren();
  const add=(title,description,state="neutral")=>{
    const item=el("div","check"),text=el("div");
    text.append(el("strong","",title),el("small","",description));
    item.append(el("span","check-icon "+state,state==="good"?"✓":state==="warn"?"!":"—"),text);$("checks").append(item);
  };
  if(!run){
    add("No run loaded","Import an exported scenario to inspect its evidence.");
    add("No pass or failure inferred","An empty chart has no result.");
    $("checks-note").textContent="The guided example remains available separately.";
    return;
  }
  add(run.example?"Historical fixture":run.status==="passed"?"Run reports passed":run.status==="failed"?"Run reports failed":"Run status not supplied",
    run.example?"Saved results from the original two-round replay.":run.status==="not reported"?"This file contains measurements without a completion report.":"Status is supplied by the imported file, not a new browser execution.",
    run.status==="failed"?"warn":run.status==="passed"?"good":"neutral");
  add("Observer agreement",row?.observer_agreement===true?(run.observerCount||4)+" observers reported the same checkpoint.":run.example&&row?.round>0?"Four stored checkpoint hashes match in the fixture.":"No per-observer agreement evidence in this record.",
    row?.observer_agreement===true?"good":"neutral");
  const stable=row?.stable_rounds;
  add("Numerical stability",stable!=null?stable+" / "+limits().stable_rounds+" consecutive rounds; score ≤ "+limits().score_delta+", share ≤ "+limits().share_delta_ppm+" ppm.":"Not measured by the two-round example.",stable>=limits().stable_rounds?"good":"neutral");
  add("Consensus progress","This reputation trace does not establish live validator enforcement or post-restart finality.");
  $("checks-note").textContent=run.example?"The example explains the mechanism; use imported multi-round results to examine convergence.":"A passed scenario is bounded evidence, not a general security or convergence proof.";
}
async function importFiles(files) {
  $("import-message").textContent="Reading files…";
  const messages=[];let last=null;
  for(const file of Array.from(files).slice(0,8)) {
    try {
      assert(file.size<=5*1024*1024,"File exceeds 5 MB.");
      const data=JSON.parse(await file.text());
      const run=data?.kind==="deterministic_checkpoint_fixture"?parseFixture(data):parseTrace(data,file.name);
      runs.set(run.scenario,run);last=run.scenario;
      messages.push(file.name+": loaded.");
    } catch(error) { messages.push(file.name+": "+error.message); }
  }
  $("import-message").textContent=messages.join(" ");
  if(last){selectScenario(last);$("import-dialog").close();notify(messages.join(" "));}
}
const liveNames=Array.from({length:4},(_,i)=>"cordial-validator-"+(i+1));
const age=value=>{const ms=Date.parse(value);return Number.isFinite(ms)?Math.max(0,(Date.now()-ms)/1000):Infinity;};
const weightIdentity=weights=>JSON.stringify(Object.entries(weights||{}).sort(([a],[b])=>a.localeCompare(b)));
function renderLive() {
  const data=latestLive, statuses=liveNames.map(name=>data?.statuses?.[name]||null), available=statuses.filter(Boolean);
  const first=available[0];
  const complete=available.length===4;
  const fresh=complete&&liveReachable&&age(data.generated_at)<=10&&liveNames.every(name=>age(data.status_updated_at?.[name])<=15);
  const minimum=complete?Math.min(...available.map(s=>s.finalized_hashes.length)):0;
  const sameRound=complete&&available.every(s=>s.active_por_round===first.active_por_round);
  const sameProjection=complete&&available.every(s=>s.active_por_checkpoint===first.active_por_checkpoint&&s.weight_commitment===first.weight_commitment&&weightIdentity(s.weights)===weightIdentity(first.weights));
  const samePrefix=complete&&available.every(s=>JSON.stringify(s.finalized_hashes.slice(0,minimum))===JSON.stringify(first.finalized_hashes.slice(0,minimum)));
  const conflict=complete&&(!samePrefix||(sameRound&&!sameProjection));
  const agreed=complete&&sameRound&&sameProjection&&samePrefix&&!data.error;
  let badge="Feed unavailable",summary="No live snapshot is available. The simulation explorer works without a cluster.";
  if(complete) {
    if(!liveReachable){badge="Disconnected";summary="Showing the last received snapshot. The live feed is unavailable.";}
    else if(conflict){badge="Reports conflict";summary="Observers disagree on a finalized prefix or on weights at the same round. Inspect their logs.";}
    else if(!fresh){badge="Saved / stale";summary="The reports are not current. They cannot establish live agreement.";}
    else if(!sameRound){badge="Catching up";summary="Observers are at different PoR rounds. Wait for a common checkpoint before comparing weights.";}
    else if(agreed){badge="Live · agreed";summary="All four observers report the same active checkpoint, weight map, and common finalized prefix.";}
    else {badge="Gate pending";summary=data.error||"Observer reports are present; the configured progress gate is pending.";}
  } else if(available.length) {badge=available.length+" / 4 reporting";summary="Waiting for all four observer reports.";}
  $("live-badge").textContent=badge;$("live-badge").className="badge"+(agreed&&fresh?" good":conflict&&fresh?" bad":"");
  $("live-summary").textContent=summary;
  $("nav-live-dot").classList.toggle("online",Boolean(agreed&&fresh));
  $("live-dot").classList.toggle("online",Boolean(agreed&&fresh));
  $("observers").replaceChildren();
  statuses.forEach((status,i)=>{
    const card=el("article","observer"),heading=el("div","observer-heading");
    heading.append(el("strong","","Observer "+(i+1)),el("span","observer-index",status?(fresh?"REPORTING":"LAST REPORT"):"NO REPORT"));
    const dl=el("dl");
    [["PoR round",status?.active_por_round],["Finalized hashes",status?.finalized_hashes.length],["Source height",status?.source_height]].forEach(([name,value])=>{
      dl.append(el("dt","",name),el("dd","",number(value)));
    });
    card.append(heading,dl,el("code","",status?short(status.active_por_checkpoint):"No checkpoint"));
    if(status?.candidate)card.append(el("code","","Candidate "+status.candidate.por_round+" · not active"));
    $("observers").append(card);
  });
  let progress="Progress not yet observed in this browser session.";
  if(agreed&&fresh) {
    if(!lastLiveProgress)lastLiveProgress={count:minimum,time:Date.now(),advanced:false};
    else if(minimum>lastLiveProgress.count)lastLiveProgress={count:minimum,time:Date.now(),advanced:true};
    else if(minimum<lastLiveProgress.count)lastLiveProgress={count:minimum,time:Date.now(),advanced:false};
    progress=minimum+" common finalized hashes · "+(lastLiveProgress.advanced?"last observed advance "+Math.floor((Date.now()-lastLiveProgress.time)/1000)+"s ago":"no new hashes observed since connection");
  }
  $("live-detail").textContent=complete?progress:"Waiting for observer reports";
  $("live-updated").textContent=data?.generated_at?"Snapshot "+new Date(data.generated_at).toLocaleTimeString():"Refreshes every 2.5 seconds";
}
function validateLive(data) {
  assert(data?.schema_version===1&&data.kind==="live_shadow_snapshot"&&data.statuses&&typeof data.statuses==="object","Invalid live snapshot.");
  assert(Number.isFinite(Date.parse(data.generated_at)),"Missing live timestamp.");
  for(const name of liveNames) {
    const s=data.statuses[name];if(!s)continue;
    assert(validNumber(s.active_por_round)&&validNumber(s.source_height)&&Array.isArray(s.finalized_hashes)&&s.finalized_hashes.every(hash=>typeof hash==="string"),"Invalid observer report.");
    assert(s.weights&&Object.values(s.weights).every(validNumber),"Invalid live weights.");
  }
}
async function pollLive() {
  try {
    const response=await fetch("por-live-shadow.json?t="+Date.now(),{cache:"no-store",signal:AbortSignal.timeout(4000)});
    if(!response.ok)throw Error("No live feed");
    const data=await response.json();validateLive(data);latestLive=data;liveReachable=true;
  } catch {liveReachable=false;}
  renderLive();setTimeout(pollLive,2500);
}
function openImport() {window.pausePorReplay?.();stop();$("import-message").textContent="";$("import-dialog").showModal();}
["import-top","import-inline","empty-import"].forEach(id=>$(id).addEventListener("click",openImport));
$("close-import").addEventListener("click",()=>$("import-dialog").close());
$("import-dialog").addEventListener("click",event=>{if(event.target===$("import-dialog")){const rect=event.target.getBoundingClientRect();if(event.clientX<rect.left||event.clientX>rect.right||event.clientY<rect.top||event.clientY>rect.bottom)event.target.close();}});
$("result-files").addEventListener("change",async event=>{await importFiles(event.target.files);event.target.value="";});
const drop=$("drop-zone");
["dragenter","dragover"].forEach(type=>drop.addEventListener(type,event=>{event.preventDefault();drop.classList.add("dragging");}));
["dragleave","drop"].forEach(type=>drop.addEventListener(type,event=>{event.preventDefault();drop.classList.remove("dragging");if(type==="drop")importFiles(event.dataTransfer.files);}));
document.querySelectorAll("[data-scenario]").forEach(button=>button.addEventListener("click",()=>selectScenario(button.dataset.scenario)));
document.querySelectorAll("[data-measure]").forEach(button=>button.addEventListener("click",()=>{measure=button.dataset.measure;render();}));
$("example-button").addEventListener("click",()=>{selectScenario("example");if(!fixture)notify("The saved fixture could not be loaded. Import por-weighted-slashing.json, or serve the dashboard over HTTP.");});
$("play").addEventListener("click",play);
$("speed").addEventListener("change",()=>{if(timer){stop();play();}});
$("round-slider").addEventListener("input",event=>{stop();selectRound(Number(event.target.value));});
$("jump-start").addEventListener("click",()=>{stop();selectRound(0);});
$("jump-end").addEventListener("click",()=>{stop();selectRound(rows().length-1);});
$("jump-fault").addEventListener("click",()=>{stop();selectRound(firstFault());});
$("trajectory").addEventListener("click",event=>{
  if(!rows().length)return;
  const rect=event.currentTarget.getBoundingClientRect(),position=(event.clientX-rect.left)/rect.width*Number(event.currentTarget.dataset.width);
  const chartLeft=Number(event.currentTarget.dataset.left),chartRight=Number(event.currentTarget.dataset.right);
  const wanted=rows()[0].round+Math.max(0,Math.min(1,(position-chartLeft)/(chartRight-chartLeft)))*(rows().at(-1).round-rows()[0].round);
  let nearest=0;rows().forEach((row,i)=>{if(Math.abs(row.round-wanted)<Math.abs(rows()[nearest].round-wanted))nearest=i;});
  stop();selectRound(nearest);
});
$("copy-checkpoint").addEventListener("click",async()=>{
  if(!currentHash)return;
  try {await navigator.clipboard.writeText(currentHash);notify("Full checkpoint hash copied.");}
  catch {notify("Clipboard access unavailable. Full checkpoint: "+currentHash);}
});
document.addEventListener("keydown",event=>{
  if($("import-dialog").open||/^(INPUT|SELECT|BUTTON|TEXTAREA|A)$/.test(event.target.tagName)||event.ctrlKey||event.metaKey||event.altKey)return;
  if ($("explorer-view").hidden) return;
  const rect=$("chart-wrap").getBoundingClientRect();if(rect.bottom<0||rect.top>window.innerHeight)return;
  if(event.key==="ArrowRight"){event.preventDefault();stop();selectRound(index+1);}
  if(event.key==="ArrowLeft"){event.preventDefault();stop();selectRound(index-1);}
  if(event.key===" "){event.preventDefault();play();}
});
document.addEventListener("visibilitychange",()=>{if(document.hidden)stop();});
let resizeFrame;
window.addEventListener("resize",()=>{cancelAnimationFrame(resizeFrame);resizeFrame=requestAnimationFrame(()=>drawChart(current(),rows()));});
async function start() {
  render();renderLive();
  try {
    const response=await fetch("por-weighted-slashing.json",{cache:"no-store"});
    if(!response.ok)throw Error("Fixture unavailable");
    const result=parseFixture(await response.json());fixture=result;runs.set("example",result);
    if(selected==="example"){index=1;render();}
  } catch {if(selected==="example")notify("The saved example could not load. Serve this directory over HTTP, or import the fixture JSON.");}
  pollLive();
}
start();
