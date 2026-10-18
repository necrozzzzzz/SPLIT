"""Compare a SourceTV extraction with a bot CSV. No game or file mutations.
Alignment uses early grounded motion only; later divergence is NOT fitted away.
Client input observations are NOT proof of server execution.
"""
import argparse,bisect,csv,json,math,statistics
from pathlib import Path

def interpolate(points,t):
    times=[x[0] for x in points]
    i=max(0,min(len(points)-2,bisect.bisect_right(times,t)-1))
    a,pa=points[i];b,pb=points[i+1];f=max(0,min(1,(t-a)/(b-a)))
    return [x+f*(y-x) for x,y in zip(pa,pb)]

def distance(a,b):return math.sqrt(sum((x-y)**2 for x,y in zip(a,b)))

def summarize_inputs(rows):
    available=[r for r in rows if r.get('input_read_started_ms')]
    result={'rowsWithInputAttempt':len(available),'errors':sum(bool(r.get('input_error')) for r in rows),
        'limitation':'Client observations can be stale/unpublished. Angles are not tied atomically to command numbers.'}
    nums=[int(r['input_last_command']) for r in available if r.get('input_last_command') and r.get('input_buffers_match')=='1']
    high=-1;rollbacks=0
    for n in nums:
        if n<high:rollbacks+=1
        high=max(high,n)
    result.update(distinctMatchingCommandNumbers=len(set(nums)),rollbackObservations=rollbacks,
        matchingBuffers=sum(r.get('input_buffers_match')=='1' for r in available),
        stableAngles=sum(r.get('input_angles_stable')=='1' for r in available),
        reflectedButtonRows=sum(r.get('button_layout')=='reflectedArray' for r in available),
        rawButtonRows=sum(r.get('button_layout')=='rawEmbeddedMember' for r in available))
    return result

def compare(source,rows,align_start=0.95,align_end=2.65):
    hz=source['tickRate']
    if not isinstance(hz,(int,float)) or hz<=0:raise ValueError('Missing valid demo tickRate')
    unique={x['tick']:x['position'] for x in source['trajectory']}
    points=[(t/hz,unique[t]) for t in sorted(unique)]
    if len(points)<2:raise ValueError('At least two reference positions required')
    bot=[]
    for r in rows:
        if r.get('read_error'):continue
        try:t=float(r['elapsed_ms'])/1000;p=[float(r['pos_'+a]) for a in 'xyz']
        except (ValueError,KeyError):continue
        if math.isfinite(t) and all(map(math.isfinite,p)):bot.append((t,p))
    if len(bot)<10:raise ValueError('Insufficient bot positions')
    def loss(offset):
        errors=[distance(p,interpolate(points,t-offset))**2 for t,p in bot if align_start<=t-offset<=align_end]
        return statistics.mean(errors) if len(errors)>=10 else math.inf
    # Deterministic coarse/fine search; no NumPy/SciPy dependency.
    best=min((i/100 for i in range(-200,401)),key=loss)
    best=min((best+i/10000 for i in range(-100,101)),key=loss)
    if not math.isfinite(loss(best)):raise ValueError('No overlap for the early alignment window')
    samples=[{'referenceSeconds':round(t-best,4),'errorUnits':round(distance(p,interpolate(points,t-best)),3),
        'botPosition':p,'referencePosition':interpolate(points,t-best)} for t,p in bot if points[0][0]<=t-best<=points[-1][0]]
    return {'offsetSeconds':best,'earlyWindowSeconds':[align_start,align_end],'earlyRmsUnits':math.sqrt(loss(best)),
        'sourceFinalPosition':points[-1][1],'botFinalPosition':bot[-1][1],
        'inputObservations':summarize_inputs(rows),'samples':samples,
        'limitations':['Positions may be interpolated and read non-atomically.','Alignment offset is estimated, not a shared engine timestamp.','No causality or input loss is inferred from positional divergence alone.']}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('source',type=Path);p.add_argument('trace',type=Path);p.add_argument('--out',type=Path,required=True)
    p.add_argument('--align-start',type=float,default=.95);p.add_argument('--align-end',type=float,default=2.65);a=p.parse_args()
    if not a.align_start<a.align_end:raise ValueError('Invalid alignment window')
    if a.out.resolve() in [a.source.resolve(),a.trace.resolve()]:raise ValueError('Output must not overwrite an input')
    result=compare(json.loads(a.source.read_text(encoding='utf-8')),list(csv.DictReader(a.trace.open(encoding='utf-8-sig'))),a.align_start,a.align_end)
    a.out.write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps({k:v for k,v in result.items() if k not in ['samples']},indent=2))
if __name__=='__main__':main()
