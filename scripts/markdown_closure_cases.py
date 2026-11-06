"""Deterministic requests for the pinned Goldmark closure-search oracle."""
from itertools import product


def closure_requests(block=False):
    requests = []
    for opener, closer in [(40,41),(91,93),(123,125),(34,34),(39,39)]:
        o,c=bytes([opener]),bytes([closer])
        sources=[b'',c,b'a'+c+b'z',o+b'x'+c+c,o+c,b'\\'+c+c,
                 b'`'+c+b'`'+c,b'``'+c+b'`x``'+c,b'`'+c,
                 b'a\n'+c,b'`x\n'+c+b'`'+c,b'\\`'+c,
                 b'\\a'+c,b'\t'+c,'😀'.encode()+c,b'\xff'+c]
        for data in sources:
            for code,nesting,newline,advance,peek in product([False,True],repeat=5):
                operation={'Kind':'closure','Opener':opener,'Closer':closer,'Options':{'CodeSpan':code,'Nesting':nesting,'Newline':newline,'Advance':advance}}
                base={'Source':data.hex(),'PeekLine':peek,'Operations':[operation,{'Kind':'save'},operation]}
                if not block:
                    requests.append(base)
                    continue
                lines=[];offset=0
                for line in data.splitlines(keepends=True):
                    lines.append({'Start':offset,'Stop':offset+len(line),'Padding':0,'ForceNewline':False});offset+=len(line)
                for padded in [False,True]:
                    segments=[{**s,'Padding':2 if padded else 0,'ForceNewline':padded} for s in lines]
                    requests.append({**base,'Block':True,'Segments':segments})
    # Every byte after a backslash qualifies the pinned ASCII punctuation test.
    for byte in range(256):
        base={'Source':bytes([92,byte,93]).hex(),'PeekLine':True,'Operations':[{'Kind':'closure','Opener':91,'Closer':93,'Options':{'CodeSpan':False,'Nesting':True,'Newline':True,'Advance':True}}]}
        if block:base.update(Block=True,Segments=[{'Start':0,'Stop':3,'Padding':0,'ForceNewline':False}])
        requests.append(base)
    return requests


def code_span_requests(block=False):
    requests=[]
    for opener,closer,body,peek in product(range(1,5),range(6),
        [b'',b' ',b'  ',b' x ',b' x',b'x ',b'\t',b'\tX\t',b'\n',b'\n x \n',b'a`b',b'a``b',b'\\`',b'${x}', '😀'.encode(),b'\xff'],[False,True]):
        data=b'`'*opener+body+b'`'*closer+b'tail'
        base={'Source':data.hex(),'PeekLine':peek,'Operations':[{'Kind':'code-span'},{'Kind':'save'}]}
        if not block:
            requests.append(base)
            continue
        lines=[];offset=0
        for line in data.splitlines(keepends=True):
            lines.append({'Start':offset,'Stop':offset+len(line),'Padding':0,'ForceNewline':False});offset+=len(line)
        for padded in [False,True]:
            segments=[{**s,'Padding':2 if padded and i>0 else 0,'ForceNewline':padded} for i,s in enumerate(lines)]
            requests.append({**base,'Block':True,'Segments':segments})
    return requests
