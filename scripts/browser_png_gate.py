"""Strict RGB/RGBA browser screenshot gate; validates every declared PNG board."""
from pathlib import Path
from PIL import Image, ImageChops


def collect_browser_pngs(output: Path):
    directory=output.with_suffix("")
    if output.is_file() and directory.exists():raise ValueError("ambiguous single and board outputs")
    if output.is_file():files={"":output}
    elif directory.is_dir():
        paths=sorted(p for p in directory.rglob("*")if p.is_file())
        if not paths or any(p.suffix.lower()!=".png"for p in paths):raise ValueError("empty or incomplete PNG board inventory")
        files={p.relative_to(directory).as_posix():p for p in paths}
    else:raise ValueError("missing PNG output")
    frames={}
    for name,path in files.items():
        with Image.open(path)as image:
            if image.format!="PNG"or image.mode not in {"RGB","RGBA"}or getattr(image,"n_frames",1)!=1:raise ValueError("unsupported screenshot encoding")
            image.verify()
        with Image.open(path)as image:
            image.load();frames[name]=image.convert("RGBA")
    return frames


def compare_browser_pngs(expected:Path,actual:Path,expected_boards:int):
    try:
        a=collect_browser_pngs(expected);b=collect_browser_pngs(actual)
        if expected_boards<=0 or len(a)!=expected_boards or len(b)!=expected_boards or a.keys()!=b.keys():raise ValueError("incomplete or unequal PNG board inventory")
        rows=[]
        for name in a:
            x,y=a[name],b[name]
            if x.size!=y.size:rows.append(dict(name=name,matched=False,expected_size=x.size,actual_size=y.size));continue
            channels=ImageChops.difference(x,y).split();mask=channels[0]
            for channel in channels[1:]:mask=ImageChops.lighter(mask,channel)
            count=x.width*x.height-mask.histogram()[0]
            rows.append(dict(name=name,matched=count==0,width=x.width,height=x.height,different_pixels=count,bounds=mask.getbbox()))
        return dict(matched=all(row['matched']for row in rows),complete=True,boards=len(rows),artifacts=rows)
    except (OSError,ValueError,SyntaxError)as error:return dict(matched=False,complete=False,error=str(error))
