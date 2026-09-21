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
    window.fixture={people:[],streams:new Set(),presenceReads:0,connections:0,revision:0,leavePending:false};
    const f=fixture;const original=window.fetch.bind(window);
    const frame=(name,data={})=>new TextEncoder().encode('event: '+name+'\\ndata: '+JSON.stringify(data)+'\\n\\n');
    const snapshot = pub => ({participants:f.people.map(p=>pub?(({tracks,...rest})=>rest)(p):p),revision:f.revision});
    window.pushRoster = people => {f.people=people;f.revision++;for(const s of f.streams)s.controller.enqueue(frame('snapshot',snapshot(s.pub)));};
    window.fetch=async (input,init={})=>{
      const path=new URL(input,location.origin).pathname;
      if(path==='/api/account/me')return Response.json({id:'account',username:'fixture',displayName:'Grok'});
      if(path==='/api/media/status')return Response.json({enabled:true});
      if(path==='/api/media/presence'){f.presenceReads++;return Response.json(snapshot(true));}
      if(path.endsWith('/events')){
        const pub=path.includes('/presence/');if(pub)f.connections++;
        let s;return new Response(new ReadableStream({start(controller){
          s={controller,pub};f.streams.add(s);
          init.signal?.addEventListener('abort',()=>{f.streams.delete(s);controller.error(new DOMException('Cancelled','AbortError'));},{once:true});
          controller.enqueue(frame('ready'));controller.enqueue(frame('snapshot',snapshot(pub)));
        },cancel(){f.streams.delete(s);}}),{headers:{'content-type':'text/event-stream'}});
      }
      if(path==='/api/media/join'){
        pushRoster([{id:'self',name:'Grok',countryCode:'US',muted:false,deafened:false,tracks:[]}]);
        return Response.json({token:'fixture-capability',id:'self',iceServers:[]});
      }
      if(path==='/api/media/publish')return Response.json({sessionDescription:{type:'answer',sdp:'v=0'}});
      if(path==='/api/media/snapshot')return Response.json(snapshot(false));
      if(path==='/api/media/state')return new Response(null,{status:204});
      if(path==='/api/media/leave'){
        f.leavePending=true;
        await new Promise(resolve=>{f.commitLeave=()=>{pushRoster([]);f.leavePending=false;resolve();};});
        return new Response(null,{status:204});
      }
      return original(input,init);
    };
    const ac=new AudioContext();await ac.resume();
    PublicCallClient.prototype.prepareMicrophone=()=>{};
    PublicCallClient.prototype.openMicrophone=async()=>ac.createMediaStreamDestination().stream.getAudioTracks()[0];
    window.RTCPeerConnection=class extends EventTarget {
      connectionState='connected';iceGatheringState='complete';senders=[];
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
    pushRoster([{id:'other',name:'Other guest',muted:true,deafened:false,tracks:[]}]);
    await until(()=>roster().includes('Other guest'));
    assert(mount.querySelector('.participant-name').textContent.includes('Muted'),'spectator mute update missing');
    pushRoster([]);await until(()=>roster().length===0);
    return {result:'PASS three join/leave races and spectator changes',presenceJsonRequests:fixture.presenceReads,streamConnections:fixture.connections};
  `));
  if (artifacts) {
    mkdirSync('.amp/in/artifacts', {recursive:true});
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/live-presence-after-leave.png`);
  }
  console.log(evaluate(`
    // Pause reconnection after EOF: old presence remains explicitly marked as stale.
    pushRoster([{id:'other',name:'Other guest',muted:false,deafened:false,tracks:[]}]);
    await until(()=>roster().includes('Other guest'));
    const fetchBefore=window.fetch;let release;
    window.fetch=(url,init)=>String(url).includes('/presence/events')?new Promise(r=>{release=()=>r(fetchBefore(url,init));}):fetchBefore(url,init);
    for(const s of [...fixture.streams])if(s.pub){fixture.streams.delete(s);s.controller.close();}
    await until(()=>mount.textContent.includes('Updating live roster'));
    window.restorePresence=async()=>{await until(()=>release);fixture.people=[];release();await until(()=>roster().length===0&&!mount.textContent.includes('Updating live roster'));};
    return 'PASS disconnected stream visibly marks the last-known roster';
  `));
  if (artifacts) {
    browser("set", "viewport", "390", "844", "2");
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/live-presence-reconnecting-mobile.png`);
  }
  console.log(evaluate(`await restorePresence();await cleanup();return 'PASS reconnection replaces stale roster with current snapshot';`));
} catch (error) {
  console.error(evaluate(`return {page:document.body.innerText,presenceJsonRequests:window.fixture?.presenceReads};`));
  throw error;
} finally {
  browser("close");
}
