// Read-only extraction. Pinned deadem 4.0.2; no game memory access.
import {createReadStream,writeFileSync} from 'node:fs';
import {Parser,InterceptorStage} from 'deadem';
const [input,output]=process.argv.slice(2);
if(!input||!output)throw Error('Usage: node extract_sourcetv.mjs INPUT.dem OUTPUT.json');
const parser=new Parser(), commands=[], trajectory=[], packets={};let tick=-1;
// deadem events share a mutable accumulator. Copy after EACH command, before
// the next command mutates it, rather than after a whole packet's batch.
const handler=parser._engine.getDemoMessageHandler();
const original=handler.handleSvcUserCommands.bind(handler);
handler.handleSvcUserCommands=(packet,snapshot=false)=>{
 for(const raw of packet.data?.commands??[]){
  const events=original({data:{commands:[raw]}},snapshot,true);
  for(const event of events){const u=event.userCommand;
   commands.push({tick,slot:u.slot,number:u.number,gap:event.gap,state:structuredClone(u.state)});
  }
 }
 return [];
};
parser.registerPreInterceptor(InterceptorStage.DEMO_PACKET,p=>{tick=p.tick;});
parser.registerPostInterceptor(InterceptorStage.DEMO_PACKET,p=>{
 packets[p.type.code]=(packets[p.type.code]??0)+1;
 const demo=parser.getDemo();
 const handle=commands.at(-1)?.state.base?.pawnEntityHandle;
 if(handle===undefined)return;
 const pawn=demo.getEntityByHandle(handle);
 if(!pawn)return;
 const position=['X','Y','Z'].map(a=>{
  const cell=pawn.getField('CBodyComponent.m_cell'+a),v=pawn.getField('CBodyComponent.m_vec'+a);
  return Number.isFinite(cell)&&Number.isFinite(v)?(cell-64)*512+v:NaN;
 });
 if(position.every(Number.isFinite)&&p.tick>=0)trajectory.push({tick:p.tick,position});
});
await parser.parse(createReadStream(input));
const result={schemaVersion:1,source:input,parser:'deadem@4.0.2',tickRate:parser.getDemo().server?.tickRate,
 packets,commands,trajectory,
 limitations:['Server packet ticks are batched, not per-command execution timestamps.','Commands and entity positions are separate streams.','A complete extraction does not prove BOTREC playback equivalence.']};
writeFileSync(output,JSON.stringify(result,(_,v)=>typeof v==='bigint'?String(v):v,2));
console.log(`Extracted ${commands.length} commands and ${trajectory.length} positions into ${output}`);
if(!commands.length)throw Error('No user-command stream in this demo; do not infer commands from entity positions.');
