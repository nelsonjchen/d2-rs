"""Fast unfiltering preserves the strict decoder's contract and pixel bytes."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import struct
import zlib
SPEC=importlib.util.spec_from_file_location('checker',Path(__file__).with_name('check-render-parity.py'))
CHECKER=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(CHECKER)
def chunk(kind,payload):
 return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
class FastDecoderTests(unittest.TestCase):
 @unittest.skipIf(CHECKER.Image is None,'Pillow fast path requires Pillow')
 def test_all_filters_and_multiline_pixels_match_fallback(self):
  pixels=bytes((i*73+19)%256 for i in range(7*5*3));stride=21
  with tempfile.TemporaryDirectory() as tmp:
   for filter_type in range(5):
    raw=bytearray();previous=bytes(stride)
    for y in range(5):
     row=pixels[y*stride:(y+1)*stride];raw.append(filter_type)
     for i,value in enumerate(row):
      left=row[i-3] if i>=3 else 0;up=previous[i];ul=previous[i-3] if i>=3 else 0
      p=left+up-ul;pa,pb,pc=abs(p-left),abs(p-up),abs(p-ul)
      predictor=[0,left,up,(left+up)//2,left if pa<=pb and pa<=pc else up if pb<=pc else ul][filter_type]
      raw.append((value-predictor)&255)
     previous=row
    p=Path(tmp)/f'{filter_type}.png';p.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',7,5,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
    fast=CHECKER.decode_png(p)
    with patch.object(CHECKER,'Image',None):slow=CHECKER.decode_png(p)
    self.assertEqual(fast,slow);self.assertEqual(fast,(7,5,pixels))
if __name__=='__main__':unittest.main()
