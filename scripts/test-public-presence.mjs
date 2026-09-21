// Real Call component/client, mocked API/WebRTC. No provider resources or real microphone.
// Run against Vite: node scripts/test-public-presence.mjs http://localhost:5174
// For a baseline comparison, place the old page in CallPresenceBefore.tsx and use --before.
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
if (!["localhost", "127.0.0.1"].includes(origin.hostname)) throw new Error("Use local Vite, not production.");
const component = process.argv.includes("--before") ? "CallPresenceBefore" : "Call";
const session = `presence-${process.pid}`;
const artifacts = process.argv.includes("--screenshots");
function browser(...args) {
  return execFileSync("agent-browser", ["--session", session, "--args", "--autoplay-policy=no-user-gesture-required", ...args], {encoding:"utf8", timeout:35_000});
}
function evaluate(code) { return browser("eval", `(async()=>{${code}})()`); }

try {
  browser("open", new URL("/live", origin).href);
  browser("set", "viewport", "1280", "900", "2");
  browser("wait", "--fn", "!!document.querySelector('.join-card')");
  console.log(evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {PublicCallClient} = await import('/src/media/client.ts');
    const {default: Call} = await import('/src/pages/${component}.tsx');
    window.assert = (condition,message) => {if(!condition)throw Error(message);};
    window.wait = ms => new Promise(r=>setTimeout(r,ms));
    window.until = async predicate => {for(let i=0;i<100;i++){if(predicate())return;await wait(10);}throw Error('Timed out waiting for roster');};
    document.querySelector('main').hidden=true;
    const label=document.createElement('p');label.textContent='Regression fixture: mocked API/WebRTC, no live participants';document.body.append(label);
    const mount=window.mount=document.createElement('div');document.body.append(mount);
    window.button = text => [...mount.querySelectorAll('button')].find(b=>b.textContent.trim()===text);
    window.roster = () => [...mount.querySelectorAll('[aria-label="People in voice"] .participant-name strong')].map(n=>n.textContent);
    window.fixture={people:[],streams:new Set(),presenceReads:0,connections:0,revision:0,leavePending:false,peers:[],subscribes:0,closes:0};
    const f=fixture;const original=window.fetch.bind(window);
    const frame=(name,data={})=>new TextEncoder().encode('event: '+name+'\\ndata: '+JSON.stringify(data)+'\\n\\n');
    const snapshot = pub => ({participants:f.people.map(p=>pub?(({tracks,...rest})=>rest)(p):p),revision:f.revision});
    window.pushRoster = people => {f.people=people;f.revision++;for(const s of f.streams)if(!s.hold)s.sendSnapshot();};
    window.fetch=async (input,init={})=>{
      const path=new URL(input,location.origin).pathname;
      if(path==='/api/account/me')return Response.json({id:'account',username:'fixture',displayName:'Grok'});
      if(path==='/api/media/status')return Response.json({enabled:true});
      if(path==='/api/media/presence'){f.presenceReads++;return Response.json(snapshot(true));}
      if(path.endsWith('/events')){
        const pub=path.includes('/presence/');if(pub)f.connections++;
        let s;return new Response(new ReadableStream({start(controller){
          s={controller,pub,hold:f.holdHandoff&&!pub,sendSnapshot:()=>controller.enqueue(frame('snapshot',snapshot(pub)))};f.streams.add(s);
          init.signal?.addEventListener('abort',()=>{f.streams.delete(s);controller.error(new DOMException('Cancelled','AbortError'));},{once:true});
          controller.enqueue(frame('ready'));if(!s.hold)s.sendSnapshot();
        },cancel(){f.streams.delete(s);}}),{headers:{'content-type':'text/event-stream'}});
      }
      if(path==='/api/media/join'){
        const intent=JSON.parse(init.body);f.joinIntent=intent;
        pushRoster([{id:'self',name:'Grok',countryCode:'US',muted:intent.muted??false,deafened:intent.deafened??false,tracks:[]}]);
        return Response.json({token:'fixture-capability',id:'self',iceServers:[]});
      }
      if(path==='/api/media/publish')return Response.json({sessionDescription:{type:'answer',sdp:'v=0'}});
      if(path==='/api/media/subscribe'){f.subscribes++;return Response.json({tracks:[{mid:'remote-'+f.subscribes}],requiresImmediateRenegotiation:false});}
      if(path==='/api/media/close'){f.closes++;return f.failClose?Response.json({error:'cleanup unavailable'},{status:503}):Response.json({});}
      if(path==='/api/media/snapshot')return Response.json(snapshot(false));
      if(path==='/api/media/state'){
        const state=JSON.parse(init.body);const self=f.people.find(p=>p.id==='self');
        if(self&&(self.muted!==state.muted||self.deafened!==state.deafened))pushRoster(f.people.map(p=>p.id==='self'?{...p,...state}:p));
        return new Response(null,{status:204});
      }
      if(path==='/api/media/leave'){
        f.leavePending=true;
        await new Promise(resolve=>{f.commitLeave=()=>{pushRoster([]);f.leavePending=false;resolve();};});
        return new Response(null,{status:204});
      }
      return original(input,init);
    };
    const ac=new AudioContext();await ac.resume();
    navigator.mediaDevices.enumerateDevices=async()=>[
      {deviceId:'default',groupId:'fixture',kind:'audioinput',label:'Mock microphone'},
      {deviceId:'default',groupId:'fixture',kind:'audiooutput',label:'Mock speakers'},
    ];
    PublicCallClient.prototype.prepareMicrophone=()=>{};
    PublicCallClient.prototype.openMicrophone=async()=>ac.createMediaStreamDestination().stream.getAudioTracks()[0];
    window.RTCPeerConnection=class extends EventTarget {
      connectionState='connected';iceGatheringState='complete';senders=[];
      constructor(){super();f.peers.push(this);}
      addTransceiver(track){const sender={track,async replaceTrack(t){this.track=t;}};this.senders.push(sender);return {mid:'0',sender};}
      async createOffer(){return {type:'offer',sdp:'v=0'};}
      async setLocalDescription(s){this.localDescription={toJSON:()=>s};}
      async setRemoteDescription(){}
      getSenders(){return this.senders;}getReceivers(){return [];}async getStats(){return new Map();}
      close(){this.connectionState='closed';}
    };
    const root=createRoot(mount);root.render(React.createElement(Call));
    window.cleanup=async()=>{root.unmount();await ac.close();};
    await until(()=>button('Join voice')&&!button('Join voice').disabled);
    return 'Fixture mounted';
  `));
  console.log(evaluate(`
    for(let cycle=0;cycle<3;cycle++){
      button('Join voice').click();await until(()=>mount.textContent.includes('You’re in General.'));
      assert(roster().some(n=>n.includes('Grok')),'joined roster missing self');
      button('Leave voice').click();await until(()=>button('Join voice')&&fixture.leavePending);
      // Force the exact bug: idle roster reads before the leave transaction commits.
      await until(()=>roster().includes('Grok'));
      const started=performance.now();fixture.commitLeave();
      await until(()=>roster().length===0);
      assert(performance.now()-started<1000,'leave waited for polling');
      console.log('Leave cycle '+cycle+' cleared without polling');
    }
    assert(fixture.presenceReads===0,'page still polls /presence');
    button('Join voice').click();await until(()=>mount.textContent.includes('You’re in General.'));
    await until(()=>mount.textContent.includes('Connection details'));
    const peer=fixture.peers.at(-1),peerCount=fixture.peers.length;
    const self=fixture.people.find(p=>p.id==='self');
    // Watch the real page throughout remote churn, not only its final state.
    let interrupted=false;
    const observer=new MutationObserver(()=>{if(!mount.textContent.includes('You’re in General.')||!mount.textContent.includes('Connection details'))interrupted=true;});
    observer.observe(mount,{subtree:true,childList:true,characterData:true});
    fixture.failClose=true;
    for(let cycle=0;cycle<3;cycle++){
      const before=fixture.subscribes;
      pushRoster([self,{id:'phone',name:'Phone',muted:true,deafened:cycle%2===0,tracks:[{id:'phone-'+cycle,kind:'microphone'}]}]);
      await until(()=>roster().includes('Phone')&&fixture.subscribes>before);
      const row=[...mount.querySelectorAll('.participant')].find(p=>p.textContent.includes('Phone'));
      assert(row.textContent.includes(cycle%2===0?'Deafened':'Muted'),'first phone roster has incorrect control state');
      const closes=fixture.closes;
      pushRoster([self]);await until(()=>!roster().includes('Phone')&&fixture.closes>closes);
      await wait(30);
      assert(fixture.peers.length===peerCount&&peer.connectionState==='connected','remote leave replaced the local peer');
    }
    observer.disconnect();assert(!interrupted,'remote leave interrupted connected UI or diagnostics');
    assert(peer.senders[0].track.enabled,'local microphone stopped');
    mount.querySelector('[aria-label="Deafen audio"]').click();
    await until(()=>mount.querySelector('[aria-label="Undeafen audio"]'));
    button('Leave voice').click();await until(()=>fixture.leavePending);fixture.commitLeave();
    await until(()=>button('Join voice'));button('Join voice').click();
    await until(()=>mount.textContent.includes('You’re in General.'));
    assert(fixture.joinIntent.muted===true&&fixture.joinIntent.deafened===true,'rejoin did not carry remembered intent');
    button('Leave voice').click();await until(()=>fixture.leavePending);fixture.commitLeave();
    await until(()=>roster().length===0);
    pushRoster([{id:'other',name:'Other guest',muted:true,deafened:false,tracks:[]}]);
    await until(()=>roster().includes('Other guest'));
    assert(mount.querySelector('.participant-name').textContent.includes('Muted'),'spectator mute update missing');
    pushRoster([]);await until(()=>roster().length===0);
    return {result:'PASS local leave races, remote departure with failed cleanup preserves connected UI/diagnostics/peer, initial rejoin mute/deafen, spectator changes',presenceJsonRequests:fixture.presenceReads,streamConnections:fixture.connections};
  `));
  if (artifacts) {
    mkdirSync('.amp/in/artifacts', {recursive:true});
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/live-presence-after-leave.png`);
  }
  console.log(evaluate(`
    // Pause reconnection after EOF, then verify a fresh snapshot replaces stale data.
    pushRoster([{id:'other',name:'Other guest',muted:false,deafened:false,tracks:[]}]);
    await until(()=>roster().includes('Other guest'));
    const fetchBefore=window.fetch;let release;
    window.fetch=(url,init)=>String(url).includes('/presence/events')?new Promise(r=>{release=()=>r(fetchBefore(url,init));}):fetchBefore(url,init);
    for(const s of [...fixture.streams])if(s.pub){fixture.streams.delete(s);s.controller.close();}
    await until(()=>release);
    window.restorePresence=async()=>{fixture.people=[];release();await until(()=>roster().length===0);};
    return 'PASS disconnected stream reconnects';
  `));
  if (artifacts) {
    browser("set", "viewport", "390", "844", "2");
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/live-presence-reconnecting-mobile.png`);
  }
  console.log(evaluate(`await restorePresence();return 'PASS reconnection replaces stale roster with current snapshot';`));
  browser("set", "viewport", "1280", "900", "2");
  console.log(evaluate(`
    button('Join voice').click();await until(()=>mount.textContent.includes('You’re in General.'));
    await until(()=>mount.querySelector('.call-diagnostics'));
    const peer=fixture.peers.at(-1),count=fixture.peers.length;
    const bounds=()=>['.stage-placeholder','.call-diagnostics','.call-controls'].map(selector=>mount.querySelector(selector).getBoundingClientRect().top);
    const before=bounds();const original=window.fetch;let release;
    window.fetch=(url,init)=>String(url).includes('/api/media/events?')?new Promise(r=>{release=()=>r(original(url,init));}):original(url,init);
    for(const s of [...fixture.streams])if(!s.pub){fixture.streams.delete(s);s.controller.close();}
    await until(()=>release);
    assert(!mount.textContent.includes('Reconnecting live updates'),'removed banner appeared');
    assert(JSON.stringify(bounds())===JSON.stringify(before),'SSE recovery shifted the layout');
    assert(mount.textContent.includes('You’re in General.'),'SSE recovery changed call phase');
    window.restoreConnectedEvents=async()=>{
      window.fetch=original;release();await until(()=>[...fixture.streams].some(s=>!s.pub));
      const self=fixture.people.find(p=>p.id==='self');
      for(const [muted,deafened] of [[true,false],[true,true],[false,false]]){
        pushRoster([self,{id:'phone',name:'Phone',muted,deafened,tracks:[]}]);
        await until(()=>roster().includes('Phone'));
        await until(()=>{
          const text=[...mount.querySelectorAll('.participant')].find(p=>p.textContent.includes('Phone')).textContent;
          return deafened?text.includes('Deafened'):muted?text.includes('Muted'):!text.includes('Deafened')&&!text.includes('Muted');
        });
      }
      assert(peer.connectionState==='connected'&&fixture.peers.length===count,'SSE recovery replaced peer');
    };
    return 'PASS live-update outage keeps connected layout stable without the removed banner';
  `));
  if (artifacts) {
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/voice-sync-recovery-desktop.png`);
    browser("set", "viewport", "390", "844", "2");
    browser("eval", "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/voice-sync-recovery-mobile.png`, "--full");
  }
  console.log(evaluate(`await restoreConnectedEvents();return 'PASS mute/deafen pushes after stream recovery without replacing voice peer';`));
  console.log(evaluate(`
    const peer=fixture.peers.at(-1),count=fixture.peers.length,original=window.fetch;
    let rejected=0;
    window.fetch=(url,init)=>{
      if(String(url).includes('/api/media/events?')&&rejected<3){
        rejected++;return Promise.resolve(Response.json({code:'api_draining'},{status:503}));
      }
      return original(url,init);
    };
    for(const s of [...fixture.streams])if(!s.pub)s.controller.enqueue(new TextEncoder().encode('event: draining\\ndata: {}\\n\\n'));
    await until(()=>rejected===1);
    // The other device changes intent while this stream is handing off.
    for(const [muted,deafened] of [[true,true],[false,false],[true,false]]){
      pushRoster(fixture.people.map(p=>p.id==='phone'?{...p,muted,deafened}:p));await wait(15);
    }
    await until(()=>[...fixture.streams].some(s=>!s.pub));
    await until(()=>{
      const row=[...mount.querySelectorAll('.participant')].find(p=>p.textContent.includes('Phone'));
      return row&&row.textContent.includes('Muted')&&!row.textContent.includes('Deafened');
    });
    assert(rejected===3,'did not exercise draining routing rejections');
    assert(peer.connectionState==='connected'&&fixture.peers.length===count,'handoff replaced voice peer');
    assert(mount.textContent.includes('You’re in General.'),'handoff changed connected phase');
    window.fetch=original;
    return 'PASS three draining-route rejections recover current mute/deafen within the fixture deadline, preserving voice peer';
  `));
  console.log(evaluate(`
    const peer=fixture.peers.at(-1),count=fixture.peers.length,latencies=[];
    for(let cycle=0;cycle<3;cycle++){
      const old=[...fixture.streams].find(s=>!s.pub);
      fixture.holdHandoff=true;
      old.controller.enqueue(new TextEncoder().encode('event: migrating\\ndata: {}\\n\\n'));
      await until(()=>[...fixture.streams].some(s=>!s.pub&&s!==old));
      const candidate=[...fixture.streams].find(s=>!s.pub&&s!==old);
      for(let i=0;i<10;i++){
        const muted=i%2===0,deafened=i%3===0,started=performance.now();
        pushRoster(fixture.people.map(p=>p.id==='phone'?{...p,muted,deafened}:p));
        await until(()=>{
          const text=[...mount.querySelectorAll('.participant')].find(p=>p.textContent.includes('Phone')).textContent;
          return deafened?text.includes('Deafened'):muted?text.includes('Muted'):!text.includes('Deafened')&&!text.includes('Muted');
        });
        latencies.push(performance.now()-started);
        assert(fixture.streams.has(old),'old stream closed before replacement snapshot');
        assert(mount.textContent.includes('You’re in General.'),'handoff changed call phase');
        await wait(100);
      }
      assert(candidate.hold,'replacement should still be awaiting a snapshot');
      fixture.holdHandoff=false;candidate.hold=false;candidate.sendSnapshot();
      await until(()=>!fixture.streams.has(old));
      assert(fixture.peers.at(-1)===peer&&fixture.peers.length===count,'planned handoff replaced voice peer');
    }
    assert(Math.max(...latencies)<250,'local mocked updates paused during planned handoff');
    await cleanup();
    return {result:'PASS continuous status delivery during three overlapping handoffs (mocked API/WebRTC)',updates:latencies.length,maxLocalPushToDomMs:Math.round(Math.max(...latencies))};
  `));
} catch (error) {
  console.error(evaluate(`return {page:document.body.innerText,presenceJsonRequests:window.fixture?.presenceReads};`));
  throw error;
} finally {
  browser("close");
}
