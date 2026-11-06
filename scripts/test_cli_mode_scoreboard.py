#!/usr/bin/env python3
import copy
import base64
import importlib.util
import io
import json
import hashlib
import zipfile
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('score',Path(__file__).with_name('cli-mode-scoreboard.py'))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)

class OriginalInventory(unittest.TestCase):
    suite='isometric'
    def setUp(self):
        score.configure(self.suite)
        self.calls=[]
        for test,count in score.original_names().items():
            for i in range(count):
                self.calls.append(dict(test=test,call=len(self.calls),input=dict(directory='/unused',args=[],env=[],canceled=False,files={}),result=dict(error='',stdout='svg',files={})))
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_exact(self):
        self.assertTrue(all(r['outcome']=='match' for r in score.compare(self.calls,self.results)))
    def test_missing_extra_duplicate(self):
        for calls in [self.calls[:-1],self.calls+[self.calls[0]],self.calls[:-1]+[self.calls[0]]]:
            with self.assertRaises(AssertionError):score.compare(calls,self.results)
    def test_no_empty_native_pass(self):
        self.assertTrue(all(r['outcome']=='mismatch' for r in score.compare(self.calls,[])))
    def test_all_result_fields_matter(self):
        for key,value in [('error','failure'),('stdout','different'),('files',{'extra.svg':{}})]:
            results=copy.deepcopy(self.results);results[0][key]=value
            self.assertEqual(score.compare(self.calls,results)[0]['outcome'],'mismatch')
    def test_expected_data_rejected(self):
        self.calls[0]['input']['result']=self.results[0]
        with self.assertRaises(AssertionError):score.compare(self.calls,self.results)

class ExportInventory(OriginalInventory):
    suite='export'
    def test_witness_does_not_credit_original_function(self):
        self.assertEqual(set(score.TESTS), {'TestUnsupportedStdoutFormatDoesNotWrite',
            'TestUnknownOutputExtensionUsesSVG', 'TestD2rsOutputSelectionWitnesses'})
        self.assertEqual(len(self.calls),16)

class NetworkInventory(OriginalInventory):
    suite='network'

class ImageInventory(OriginalInventory):
    suite='image'
    def setUp(self):
        super().setUp()
        for call in self.calls[:9]:
            call['input']={'format':'.png','scale_bits':'0000000000000000'}
            call['result']={'format':'png','width':1600,'height':1000,'error':'invalid scale'}
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_returned_dimensions_and_cli_preservation(self):
        for index, key, value in [(0,'width',1),(0,'height',1),(0,'format','gif'),
                                   (0,'error','other'),(9,'files',{'out.png':None}),
                                   (9,'stdout','unexpected')]:
            results=copy.deepcopy(self.results);results[index][key]=value
            self.assertEqual(score.compare(self.calls,results)[0]['outcome'],'mismatch')

class ImageExportInventory(OriginalInventory):
    suite='image-export'
    def setUp(self):
        super().setUp()
        from PIL import Image
        stream=io.BytesIO();Image.new('RGB',(2,2),(1,2,3)).save(stream,format='PNG')
        self.png=stream.getvalue()
        stream=io.BytesIO()
        Image.new('RGB',(2,2),'red').save(stream,format='GIF',save_all=True,
            append_images=[Image.new('RGB',(2,2),'blue')],duration=[100,200],loop=0)
        self.gif=stream.getvalue()
        for call in self.calls:
            call['input']['args']=['input.d2','--stdout-format=gif','-']
            call['result']={'error':'','stdout':base64.b64encode(self.gif).decode(),'files':{}}
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_all_result_fields_matter(self):
        for key,value in [('error','failure'),('stdout',base64.b64encode(b'bad').decode()),('files',{'extra.svg':None})]:
            results=copy.deepcopy(self.results);results[0][key]=value
            self.assertEqual(score.compare(self.calls,results)[0]['outcome'],'mismatch')
    def png_case(self):
        call=self.calls[0]
        call['input']['args']=['input.d2','views.png']
        files={'views/index.png':{'data':base64.b64encode(self.png).decode(),'mode':0o600}}
        manifest={'format':'d2-board-output','version':1,
            'files':[{'path':'index.png','sha256':hashlib.sha256(self.png).hexdigest()}],'directories':[]}
        files['views/.d2-board-output-manifest-v1.json']={'data':base64.b64encode(json.dumps(manifest).encode()).decode(),'mode':0o600}
        call['result']={'error':'','stdout':'','files':files}
        return call,copy.deepcopy(call['result'])
    def test_png_complete_gate(self):
        call,actual=self.png_case()
        self.assertTrue(score.image_export_comparison(call,actual))
        for name in ['views/index.png','views/.d2-board-output-manifest-v1.json']:
            broken=copy.deepcopy(actual);del broken['files'][name]
            self.assertFalse(score.image_export_comparison(call,broken))
            broken=copy.deepcopy(actual);broken['files'][name]['data']=base64.b64encode(b'broken').decode()
            self.assertFalse(score.image_export_comparison(call,broken))
        broken=copy.deepcopy(actual);broken['files']['views/index.png']['mode']=0o644
        self.assertFalse(score.image_export_comparison(call,broken))
    def test_png_stdout(self):
        call=self.calls[0]
        call['input']['args']=['input.d2','--stdout-format=png','-']
        call['result']={'error':'','stdout':base64.b64encode(self.png).decode(),'files':{}}
        self.assertTrue(score.image_export_comparison(call,copy.deepcopy(call['result'])))
    def test_equal_corrupt_or_empty_images_cannot_pass(self):
        call,actual=self.png_case()
        for value in [b'',b'broken']:
            for result in [call['result'],actual]:
                result['files']['views/index.png']['data']=base64.b64encode(value).decode()
            self.assertFalse(score.image_export_comparison(call,actual))
        for value in [b'',self.gif[:-1]]:
            call=self.calls[1];call['result']['stdout']=base64.b64encode(value).decode()
            self.assertFalse(score.image_export_comparison(call,copy.deepcopy(call['result'])))
    def test_local_input_artwork_is_preserved_separately_from_outputs(self):
        call, actual = self.png_case()
        for name in ['mark.png', 'mark.gif']:
            record = {'data': base64.b64encode(b'original local artwork').decode(), 'mode': 0o600}
            call['input']['files'][name] = copy.deepcopy(record)
            call['result']['files'][name] = copy.deepcopy(record)
            actual['files'][name] = copy.deepcopy(record)
        self.assertTrue(score.image_export_comparison(call, actual))
        for field, value in [('data', ''), ('mode', 0o644)]:
            broken = copy.deepcopy(actual)
            broken['files']['mark.png'][field] = value
            self.assertFalse(score.image_export_comparison(call, broken))
        # An unrecorded emitted image cannot be mistaken for original artwork.
        del call['input']['files']['mark.png']
        self.assertFalse(score.image_export_comparison(call, actual))
    def test_gif_timing_frames_loop_and_pixels(self):
        from PIL import Image
        for options in [dict(duration=[100,300],loop=0),dict(duration=[100,200],loop=1),
                        dict(duration=[100,200],loop=0,color='green')]:
            options=options.copy();color=options.pop('color','blue')
            stream=io.BytesIO();Image.new('RGB',(2,2),'red').save(stream,format='GIF',save_all=True,
                append_images=[Image.new('RGB',(2,2),color)],**options)
            broken=copy.deepcopy(self.results);broken[0]['stdout']=base64.b64encode(stream.getvalue()).decode()
            self.assertEqual(score.compare(self.calls,broken)[0]['outcome'],'mismatch')

class SVGExportInventory(OriginalInventory):
    suite='svg-export'
    svg=b'<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1"/></svg>'
    def setUp(self):
        super().setUp()
        for call in self.calls:
            call['input']['args']=['input.d2','--stdout-format=svg','-']
            call['result']={'error':'','stdout':base64.b64encode(self.svg).decode(),'files':{}}
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_svg_file_inventory_bytes_and_modes(self):
        call=self.calls[0]
        call['input']['args']=['input.d2','views.svg']
        call['result']={'error':'','stdout':'','files':{
            'views/index.svg':{'data':base64.b64encode(self.svg).decode(),'mode':0o600},
            'views/child.svg':{'data':base64.b64encode(self.svg).decode(),'mode':0o600}}}
        self.assertTrue(score.svg_export_comparison(call,copy.deepcopy(call['result'])))
        for name in call['result']['files']:
            actual=copy.deepcopy(call['result']);del actual['files'][name]
            self.assertFalse(score.svg_export_comparison(call,actual))
            actual=copy.deepcopy(call['result']);actual['files'][name]['mode']=0o644
            self.assertFalse(score.svg_export_comparison(call,actual))
        actual=copy.deepcopy(call['result']);actual['stdout']=base64.b64encode(b'leaked').decode()
        self.assertFalse(score.svg_export_comparison(call,actual))
    def test_equal_empty_corrupt_or_geometryless_output_fails(self):
        call=self.calls[0]
        for payload in [b'',b'bad',b'<svg/>',b'<svg xmlns="http://www.w3.org/2000/svg"/>']:
            call['result']['stdout']=base64.b64encode(payload).decode()
            self.assertFalse(score.svg_export_comparison(call,copy.deepcopy(call['result'])))
        call['input']['args']=['input.d2','views.svg']
        call['result']={'error':'','stdout':'','files':{}}
        self.assertFalse(score.svg_export_comparison(call,copy.deepcopy(call['result'])))
    def test_failure_preserves_sentinel_and_emits_no_stdout(self):
        call=self.calls[0]
        call['result']={'error':'err: invalid artwork\n','stdout':'','files':{
            'previous.svg':{'data':base64.b64encode(b'previous reviewed SVG').decode(),'mode':0o600}}}
        actual=copy.deepcopy(call['result'])
        self.assertTrue(score.svg_export_comparison(call,actual))
        actual['files']['previous.svg']['data']=''
        self.assertFalse(score.svg_export_comparison(call,actual))
        call['result']['stdout']=base64.b64encode(self.svg).decode()
        self.assertFalse(score.svg_export_comparison(call,copy.deepcopy(call['result'])))

class SVGPublicationInventory(SVGExportInventory):
    suite='svg-publication'

class SourceGIFInventory(ImageExportInventory):
    suite='source-gif'
    def test_original_source_and_flag_calls(self):
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(set(score.TESTS), {'TestIsometricSourceGIFUsesSameAnimationAsFlag'})
    def test_native_entry_points_must_have_equal_encoded_bytes(self):
        # An added comment leaves every GIF frame/timing/loop unchanged.
        changed = self.gif[:-1] + b'\x21\xfe\x03tag\x00' + self.gif[-1:]
        results = copy.deepcopy(self.results)
        results[1]['stdout'] = base64.b64encode(changed).decode()
        self.assertTrue(score.image_export_comparison(self.calls[1], results[1]))
        self.assertEqual(score.compare(self.calls, results)[0]['outcome'], 'mismatch')

class SurfaceIconsInventory(SVGExportInventory):
    suite='surface-icons'
    def test_original_four_calls(self):
        self.assertEqual(len(self.calls), 4)
        self.assertEqual(set(score.TESTS), {'TestIsometricImageLocalSurfaceIconsWithoutExecutables'})
    def test_both_formats_use_their_complete_gate(self):
        from PIL import Image
        call = self.calls[0]
        call['input']['args'] = ['input.d2', '--stdout-format=png', '-']
        stream = io.BytesIO()
        Image.new('RGB', (2, 2), (255, 0, 180)).save(stream, format='PNG')
        call['result']['stdout'] = base64.b64encode(stream.getvalue()).decode()
        self.assertTrue(score.surface_icon_comparison(call, copy.deepcopy(call['result'])))
        for payload in [b'', b'bad', self.svg]:
            call['result']['stdout'] = base64.b64encode(payload).decode()
            self.assertFalse(score.surface_icon_comparison(call, copy.deepcopy(call['result'])))
        call['input']['args'] = ['input.d2', '--stdout-format=svg', '-']
        self.assertTrue(score.surface_icon_comparison(call, copy.deepcopy(call['result'])))

class PagedExportInventory(OriginalInventory):
    suite='paged-export'
    pdf=b'%PDF-1.7\n/Type /Page\n/Subtype /Image\n%%EOF\n'
    def setUp(self):
        super().setUp()
        for call in self.calls:
            call['input']['args']=['input.d2','--stdout-format=pdf','-']
            call['result']={'error':'','stdout':base64.b64encode(self.pdf).decode(),'files':{}}
        for call in self.calls[-2:]:
            call['input']={'operation':'rebase','links':[],'source_root':'root.layers.parent','pages':{}}
            call['result']={'links':[],'input_links':[],'error':''}
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_document_corruption_and_empty_inventory(self):
        call=self.calls[0]
        for data in [b'',b'bad',self.pdf[:-6],b'%PDF-1.7\n%%EOF']:
            call['result']['stdout']=base64.b64encode(data).decode()
            self.assertFalse(score.paged_export_comparison(call,copy.deepcopy(call['result'])))
        call['input']['args']=['input.d2','pages.pdf']
        call['result']={'error':'','stdout':'','files':{}}
        self.assertFalse(score.paged_export_comparison(call,copy.deepcopy(call['result'])))
    def test_file_document_bytes_modes_and_stdout(self):
        call=self.calls[0];call['input']['args']=['input.d2','pages.pdf']
        call['result']={'error':'','stdout':'','files':{'pages.pdf':{'data':base64.b64encode(self.pdf).decode(),'mode':0o600}}}
        self.assertTrue(score.paged_export_comparison(call,copy.deepcopy(call['result'])))
        for key,value in [('data',base64.b64encode(self.pdf+b'changed').decode()),('mode',0o644)]:
            actual=copy.deepcopy(call['result']);actual['files']['pages.pdf'][key]=value
            self.assertFalse(score.paged_export_comparison(call,actual))
        actual=copy.deepcopy(call['result']);actual['stdout']='leaked'
        self.assertFalse(score.paged_export_comparison(call,actual))
    def test_pptx_complete_page_image_xml(self):
        from PIL import Image
        png=io.BytesIO();Image.new('RGB',(2,2),'red').save(png,format='PNG')
        members={'ppt/presentation.xml':b'<presentation/>','ppt/slides/slide1.xml':b'<slide/>',
                 'ppt/media/slide1Image.png':png.getvalue()}
        call=self.calls[0];call['input']['args']=['input.d2','--stdout-format=pptx','-']
        def compare(members):
            buffer=io.BytesIO()
            with zipfile.ZipFile(buffer,'w') as archive:
                for name,data in members.items(): archive.writestr(name,data)
            call['result']['stdout']=base64.b64encode(buffer.getvalue()).decode()
            return score.paged_export_comparison(call,copy.deepcopy(call['result']))
        self.assertTrue(compare(members))
        for name in members:
            broken=members.copy();del broken[name]
            self.assertFalse(compare(broken))
            broken=members.copy();broken[name]=b'corrupt'
            self.assertFalse(compare(broken))
    def test_rebase_errors_and_source_preservation(self):
        for key,value in [('links',None),('input_links',[{'Target':'changed'}]),('error','wrong')]:
            results=copy.deepcopy(self.results);results[-1][key]=value
            self.assertEqual(score.compare(self.calls,results)[-1]['outcome'],'mismatch')

class SVGLinkInventory(OriginalInventory):
    suite='svg-links'
    def setUp(self):
        super().setUp()
        for i,call in enumerate(self.calls):
            board={'root':{'link':''},'shapes':[{'link':'root.layers.child','tooltip':'preserve'}],
                   'connections':None,'layers':None,'scenarios':None,'steps':None}
            call['input']={'operation':'svg-links','board':board,'key':'root','source_root':'root',
                           'paths':{'root':'/export/index.svg','root.layers.child':'/export/root.svg'},
                           'render':None if i == 0 else {'format':'svg','width':160,'height':100}}
            rebased=copy.deepcopy(board);rebased['shapes'][0]['link']='./root.svg'
            call['result']={'board':rebased,'input_board':copy.deepcopy(board),'error':'',
                'svg':None if i == 0 else '<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1"/></svg>',
                'render_error':''}
        self.results=[copy.deepcopy(c['result']) for c in self.calls]
    def test_all_result_fields_matter(self):
        for index,key,value in [(0,'error','wrong'),(0,'board',{}),(0,'input_board',{}),
                                (1,'svg','bad'),(1,'render_error','failure')]:
            results=copy.deepcopy(self.results);results[index][key]=value
            self.assertEqual(score.compare(self.calls,results)[index>0]['outcome'],'mismatch')
    def test_source_preservation_is_independent(self):
        call=copy.deepcopy(self.calls[0]);call['result']['input_board']['root']['link']='changed'
        self.assertFalse(score.svg_link_comparison(call,call['result']))
    def test_equal_corrupt_or_empty_svg_fails(self):
        for data in ['', '<svg/>', '<svg xmlns="http://www.w3.org/2000/svg"/>', '<svg']:
            call=copy.deepcopy(self.calls[1]);call['result']['svg']=data
            self.assertFalse(score.svg_link_comparison(call,call['result']))
    def test_unrelated_target_fields_are_exact(self):
        results=copy.deepcopy(self.results);results[0]['board']['shapes'][0]['tooltip']='lost'
        self.assertEqual(score.compare(self.calls,results)[0]['outcome'],'mismatch')
    def test_boolean_cannot_replace_numeric_target(self):
        call=copy.deepcopy(self.calls[0]);call['result']['board']['width']=1
        actual=copy.deepcopy(call['result']);actual['board']['width']=True
        self.assertFalse(score.svg_link_comparison(call,actual))
    def test_extra_result_does_not_pass(self):
        self.assertTrue(all(r['outcome']=='mismatch' for r in score.compare(self.calls,self.results+[self.results[0]])))

if __name__=='__main__':unittest.main()
