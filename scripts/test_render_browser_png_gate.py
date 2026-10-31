import tempfile,unittest
from pathlib import Path
from PIL import Image
from browser_png_gate import compare_browser_pngs


class BrowserPngGate(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
    def png(self,name,mode="RGBA",color=(1,2,3,255)):
        p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);Image.new(mode,(2,3),color).save(p);return p
    def test_rgb_and_rgba_compare_all_color_channels(self):
        a=self.png('a.png',"RGB",(1,2,3));b=self.png('b.png');self.assertTrue(compare_browser_pngs(a,b,1)['matched'])
        self.png('b.png',color=(9,2,3,255));r=compare_browser_pngs(a,b,1);self.assertFalse(r['matched']);self.assertEqual(r['artifacts'][0]['different_pixels'],6)
    def test_alpha_differences_fail(self):
        a=self.png('a.png');b=self.png('b.png',color=(1,2,3,254));self.assertFalse(compare_browser_pngs(a,b,1)['matched'])
    def test_missing_and_empty_outputs_fail(self):
        a=self.png('a.png');self.assertFalse(compare_browser_pngs(a,self.root/'missing.png',1)['complete']);(self.root/'empty').mkdir();self.assertFalse(compare_browser_pngs(a,self.root/'empty.png',1)['complete'])
    def test_identically_corrupt_files_fail(self):
        a=self.root/'a.png';b=self.root/'b.png';a.write_bytes(b'bad');b.write_bytes(b'bad');self.assertFalse(compare_browser_pngs(a,b,1)['matched'])
    def test_every_board_is_validated(self):
        self.png('a/one.png');self.png('b/one.png');self.png('a/two.png');self.png('b/two.png',color=(7,8,9,255));r=compare_browser_pngs(self.root/'a.png',self.root/'b.png',2);self.assertTrue(r['complete']);self.assertFalse(r['matched']);self.assertEqual(r['boards'],2)
    def test_missing_or_extra_board_fails(self):
        self.png('a/one.png');self.png('a/two.png');self.png('b/one.png');self.assertFalse(compare_browser_pngs(self.root/'a.png',self.root/'b.png',2)['complete'])
    def test_corrupt_later_board_fails(self):
        self.png('a/one.png');self.png('b/one.png');self.png('a/two.png');(self.root/'b/two.png').write_bytes(b'bad');self.assertFalse(compare_browser_pngs(self.root/'a.png',self.root/'b.png',2)['complete'])
    def test_ambiguous_or_non_png_inventory_fails(self):
        a=self.png('a.png');self.png('a/one.png');b=self.png('b.png');self.assertFalse(compare_browser_pngs(a,b,1)['complete']);self.png('c/one.png');(self.root/'c/other.txt').write_text('bad');self.assertFalse(compare_browser_pngs(b,self.root/'c.png',1)['complete'])


if __name__=='__main__':unittest.main()
