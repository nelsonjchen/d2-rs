#!/usr/bin/env python3
"""Original CLI mode/export/network/image calls, exact outputs and errors."""
import argparse
import base64
import collections
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile
import io

import references
ROOT = references.ROOT
_language_spec = importlib.util.spec_from_file_location('language_scoreboard', ROOT/'scripts/language-scoreboard.py')
LANGUAGE = importlib.util.module_from_spec(_language_spec)
_language_spec.loader.exec_module(LANGUAGE)
PACKAGE = 'd2cli'
TESTS = {'TestIsometricModePrecedence': 9, 'TestIsometricDefaultSVGAndSourceParity': 3,
         'TestIsometricModeFailures': 9, 'TestIsometricSourceConfigAppliesToSelectedBoards': 4,
         'TestIsometricSVGScaleIsIndependentOfRasterLimits': 4}
ISOMETRIC_TESTS = dict(TESTS)
FILES = ['input.d2', 'input.svg', 'out.svg', 'out.html', 'out.txt']
SUITE = 'isometric'
EXPORT_WITNESSES = ['upper_png', 'upper_pdf', 'upper_gif', 'upper_pptx',
                    'text_alias', 'ascii_alias', 'empty_stdout',
                    'empty_stdout_text', 'empty_stdout_unknown', 'explicit_mixed_case', 'replacement']

def configure(suite):
    global SUITE, TESTS
    SUITE = suite
    TESTS = dict(ISOMETRIC_TESTS)
    if suite == 'export':
        TESTS = {'TestUnsupportedStdoutFormatDoesNotWrite': 3,
                 'TestUnknownOutputExtensionUsesSVG': 2,
                 'TestD2rsOutputSelectionWitnesses': len(EXPORT_WITNESSES)}
    elif suite == 'network':
        TESTS = {'TestHelpWithPrivateNetworkEnvironment': 1,
                 'TestPrivateNetworkEnvDefaultRejectsInvalidValue': 1}
    elif suite == 'image':
        TESTS = {'TestIsometricImageOptionsAndFailures': 19,
                 'TestD2rsSourceImageAdmissionPriority': 2}
    elif suite == 'image-export':
        TESTS = {'TestIsometricImageMultiboardExports': 3,
                 'TestIsometricImageOptInKeepsNativePNG': 1,
                 'TestIsometricImageNativeCLI': 2}
    elif suite == 'svg-export':
        TESTS = {'TestIsometricSVGCLIStaticAndStdout': 2,
                 'TestIsometricSVGCLIStaticBoards': 3,
                 'TestIsometricSVGBoardNamedRootUsesFileLinks': 1,
                 'TestIsometricSVGFailurePreservesOutput': 6}
    elif suite == 'svg-publication':
        TESTS = {'TestIsometricMultiboardPublicationIsTransactional': 3,
                 'TestIsometricMultiboardEscapesUnsafeNames': 1}
    elif suite == 'paged-export':
        TESTS = {'TestIsometricPagedCLIImagesLinksNavigationAndStdout': 5,
                 'TestIsometricPagedBoardKindsFoldersAndSelection': 8,
                 'TestIsometricPagedSelectedSubtreeKeepsInternalLinks': 4}
    elif suite == 'surface-icons':
        TESTS = {'TestIsometricImageLocalSurfaceIconsWithoutExecutables': 4}
    elif suite == 'svg-links':
        TESTS = {'TestIsometricSVGBoardLinksPreserveSource': 1,
                 'TestIsometricSVGResolvedFileLinksRetainURLProvenance': 4}
    elif suite == 'source-gif':
        TESTS = {'TestIsometricSourceGIFUsesSameAnimationAsFlag': 2}

def original_names():
    if SUITE == 'svg-links':
        return collections.Counter(['TestIsometricSVGBoardLinksPreserveSource'] +
            ['TestIsometricSVGResolvedFileLinksRetainURLProvenance/'+name
             for name in ['root', 'root.layers', 'root.scenarios', 'root.steps']])
    if SUITE == 'source-gif':
        return collections.Counter({'TestIsometricSourceGIFUsesSameAnimationAsFlag': 2})
    if SUITE == 'surface-icons':
        return collections.Counter(
            ['TestIsometricImageLocalSurfaceIconsWithoutExecutables/'+name
             for name in ['mark.svg', 'mark.png'] for _ in range(2)])
    if SUITE == 'paged-export':
        return collections.Counter(
            ['TestIsometricPagedCLIImagesLinksNavigationAndStdout'] +
            ['TestIsometricPagedCLIImagesLinksNavigationAndStdout/'+format+'/stdout='+value
             for format in ['pdf','pptx'] for value in ['false','true']] +
            ['TestIsometricPagedBoardKindsFoldersAndSelection']*2 +
            ['TestIsometricPagedBoardKindsFoldersAndSelection/'+format+'/'+name
             for format in ['pdf','pptx'] for name in
             ['all_board_kinds','selected_layer','folder-only_ancestors']] +
            ['TestIsometricPagedSelectedSubtreeKeepsInternalLinks']*4)
    if SUITE == 'svg-export':
        return collections.Counter(
            ['TestIsometricSVGCLIStaticAndStdout']*2 +
            ['TestIsometricSVGCLIStaticBoards']*3 +
            ['TestIsometricSVGBoardNamedRootUsesFileLinks'] +
            ['TestIsometricSVGFailurePreservesOutput/'+name for name in
             ['invalid_artwork', 'annotation_budget', 'invalid_scale'] for _ in range(2)])
    if SUITE in {'network', 'image-export', 'svg-publication'}:
        return collections.Counter(TESTS)
    if SUITE == 'image':
        return collections.Counter(['TestIsometricImageOptionsAndFailures']*9 +
            ['TestIsometricImageOptionsAndFailures/'+name for name in
             ['wrong_format','HTML_filename','SVG_slideshow','multiple_SVG_stdout',
              'PNG_slideshow','negative_slideshow','appendix','tiny_size','source_sketch','multiple_PNG_stdout']] +
            ['TestD2rsSourceImageAdmissionPriority/'+name for name in ['scale','appendix']])
    if SUITE == 'export':
        return collections.Counter(
            ['TestUnsupportedStdoutFormatDoesNotWrite/'+name for name in ['html','HTML','unknown']]
            + ['TestUnknownOutputExtensionUsesSVG/'+name for name in ['out.html','out.unknown']]
            + ['TestD2rsOutputSelectionWitnesses/'+name for name in EXPORT_WITNESSES])
    groups = [
        ('TestIsometricModePrecedence', ['source','source_disabled','flag','flag_disables_source','flag_enables_source','environment','environment_disables_source','flag_overrides_environment','flag_enables_over_environment']),
        ('TestIsometricModeFailures', ['source_sketch','flag_sketch','source_unsupported_format','source_HTML_filename','source_zero_scale','source_non-finite_scale','source_appendix','source_dark_theme','flag_dark_theme']),
        ('TestIsometricSourceConfigAppliesToSelectedBoards', ['#00','layers.detail','scenarios.alternate','steps.next']),
        ('TestIsometricSVGScaleIsIndependentOfRasterLimits', ['.001','.001','10','10']),
    ]
    return collections.Counter([test+'/'+name for test,names in groups for name in names] + ['TestIsometricDefaultSVGAndSourceParity']*3)

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

@contextlib.contextmanager
def paged_clock():
    """Install the established oracle-only shared clock; restore exact prior bytes."""
    if SUITE != 'paged-export':
        yield
        return
    source = references.verify_source()
    directory = references.TRACE_CHECKOUT/'lib/pdf'
    original = (source/'lib/pdf/pdf.go').read_bytes()
    token = b'newGofPDF.SetMargins(0, 0, 0)'
    assert original.count(token) == 1
    controlled = original.replace(token, token+b'\n\td2rsPDFClock(newGofPDF)')
    targets = {directory/'pdf.go': controlled}
    for name, target in [('pdf_clock.go','d2rs_clock.go'), ('pdf_clock_test.go','d2rs_clock_test.go')]:
        targets[directory/target] = (ROOT/'tools/cli-scoreboard'/name).read_bytes()
    previous = {path: path.read_bytes() if path.exists() else None for path in targets}
    assert previous[directory/'pdf.go'] in [original, controlled], 'concurrent PDF mutation'
    assert all(previous[path] in [None, data] for path,data in targets.items() if path.name != 'pdf.go')
    try:
        for path,data in targets.items(): path.write_bytes(data)
        yield
    finally:
        for path,data in previous.items():
            if data is None: path.unlink(missing_ok=True)
            else: path.write_bytes(data)

def paged_export_comparison(call, actual):
    """Full document bytes with nonempty, structurally valid emitted artifacts."""
    expected = call['result']
    if actual != expected:
        return False
    if call['input'].get('operation') == 'rebase':
        return True
    if actual['error']:
        return not base64.b64decode(actual['stdout'], validate=True)
    args = call['input']['args']
    if args[-1].endswith('.png'):
        # Includes every board and its publication manifest, using the shared gate.
        return image_export_comparison(call, actual)
    if args[-1] == '-' and '--stdout-format=svg' in args:
        return svg_export_comparison(call, actual)
    try:
        if args[-1] == '-':
            format = next(a.split('=',1)[1] for a in args if a.startswith('--stdout-format='))
            data = base64.b64decode(actual['stdout'], validate=True)
        else:
            if actual['stdout']:
                return False
            format = Path(args[-1]).suffix[1:]
            data = base64.b64decode(actual['files'][args[-1]]['data'], validate=True)
        if format == 'pdf':
            return (data.startswith(b'%PDF-') and data.rstrip().endswith(b'%%EOF')
                    and b'/Type /Page\n' in data and b'/Subtype /Image' in data)
        if format != 'pptx':
            return False
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            if archive.testzip() is not None:
                return False
            names = archive.namelist()
            if not {'ppt/presentation.xml','ppt/slides/slide1.xml','ppt/media/slide1Image.png'} <= set(names):
                return False
            for name in names:
                if name.endswith(('.xml','.rels')): ET.fromstring(archive.read(name))
                if name.startswith('ppt/media/') and name.endswith('.png'):
                    from PIL import Image
                    with Image.open(io.BytesIO(archive.read(name))) as image:
                        image.load()
                        if image.width <= 0 or image.height <= 0: return False
        return True
    except (ValueError, KeyError, OSError, StopIteration, zipfile.BadZipFile, ET.ParseError):
        return False

def svg_export_comparison(call, actual):
    """Exact bytes plus nonempty, parseable geometry for every successful SVG."""
    expected = call['result']
    if actual != expected:
        return False
    try:
        stdout = base64.b64decode(actual['stdout'], validate=True)
        if actual['error']:
            # Failure preservation compares all previous bytes/modes above.
            return not stdout
        emitted = []
        before = call['input']['files']
        for name, record in actual['files'].items():
            if record is not None and name.endswith('.svg') and record != before.get(name):
                emitted.append(base64.b64decode(record['data'], validate=True))
        if call['input']['args'][-1] == '-':
            emitted.append(stdout)
        elif stdout:
            return False
        if not emitted:
            return False
        for data in emitted:
            root = ET.fromstring(data)
            if root.tag != '{http://www.w3.org/2000/svg}svg':
                return False
            if not any(node.tag in {'{http://www.w3.org/2000/svg}path',
                                    '{http://www.w3.org/2000/svg}polygon'} for node in root.iter()):
                return False
    except (ValueError, ET.ParseError):
        return False
    return True

def file_state(directory):
    names = set(FILES) | {str(p.relative_to(directory)) for p in directory.rglob('*') if p.is_file()}
    return {name: {'data':base64.b64encode((directory/name).read_bytes()).decode(), 'mode':(directory/name).stat().st_mode & 0o777}
            if (directory/name).is_file() else None for name in sorted(names)}

def image_export_comparison(call, actual):
    """Keep inventories/modes/errors exact; validate every image before comparing.

    PNG uses the shared complete-output gate. GIF compares every composited
    RGBA frame, canvas, delay and loop, as in the full CLI scoreboard.
    """
    expected = call['result']
    if set(actual) != set(expected) or actual['error'] != expected['error']:
        return False
    if actual['files'].keys() != expected['files'].keys():
        return False
    with tempfile.TemporaryDirectory(prefix='d2rs-mode-images-') as temporary:
        roots = [Path(temporary)/side for side in ['go', 'rust']]
        for root, result in zip(roots, [expected, actual]):
            root.mkdir()
            for name, record in result['files'].items():
                other = expected['files'][name]
                if (record is None) != (other is None):
                    return False
                if record is None:
                    continue
                if record['mode'] != other['mode']:
                    return False
                path = root/name
                assert not Path(name).is_absolute() and '..' not in Path(name).parts
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(base64.b64decode(record['data'], validate=True))
            (root/'stdout').write_bytes(base64.b64decode(result['stdout'], validate=True))
        def load(name):
            spec = importlib.util.spec_from_file_location('mode_'+name, ROOT/'scripts'/name)
            module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
            return module
        raster = load('check-render-parity.py')
        cli = load('cli-scoreboard.py')
        args = call['input']['args']
        destination = args[-1]
        if destination == '-':
            formats = [arg.split('=', 1)[1] for arg in args if arg.startswith('--stdout-format=')]
            if len(formats) != 1:
                return False
            image_format = formats[0]
            paths = [root/('stdout.'+image_format) for root in roots]
            for root, path in zip(roots, paths):
                (root/'stdout').rename(path)
        else:
            image_format = Path(destination).suffix[1:]
            paths = [root/destination for root in roots]
            if expected['stdout'] != actual['stdout']:
                return False
        try:
            verified_files = set()
            if image_format == 'png':
                if not raster.compare_png_outputs(*paths)['matched']:
                    return False
                # Enforce exact RGBA samples too; the shared gate reports RGB.
                from PIL import Image
                inventories = [raster.collect_png_outputs(path) for path in paths]
                for name in inventories[0]:
                    image_path = paths[0] if name == '' else paths[0].with_suffix('')/name
                    verified_files.add(image_path.relative_to(roots[0]).as_posix())
                    if name == raster.BOARD_OUTPUT_MANIFEST:
                        continue
                    images = [path if name == '' else path.with_suffix('')/name for path in paths]
                    values = []
                    for image in images:
                        with Image.open(image) as decoded:
                            decoded.load()
                            values.append((decoded.size, decoded.convert('RGBA').tobytes()))
                    if values[0] != values[1]:
                        return False
            elif image_format == 'gif':
                if cli.gif_frames(paths[0]) != cli.gif_frames(paths[1]):
                    return False
                verified_files.add(paths[0].relative_to(roots[0]).as_posix())
            else:
                return False
        except (OSError, ValueError, EOFError):
            return False
        for name, record in expected['files'].items():
            if record is None or name in verified_files:
                continue
            # Local raster artwork is an input, not another emitted board.
            # Preserve its complete original bytes/mode on both sides before
            # excluding it from the output-image inventory check.
            if record == call['input']['files'].get(name):
                if record != actual['files'][name]:
                    return False
                continue
            if name.endswith(('.png', '.gif', '/'+raster.BOARD_OUTPUT_MANIFEST)):
                return False  # No image outside the complete checked inventory.
            if record != actual['files'][name]:
                return False
    return True

def compare(calls, results):
    assert len(calls) == sum(TESTS.values()), 'missing/extra original calls'
    assert [c['call'] for c in calls] == list(range(len(calls))), 'duplicate/reordered calls'
    assert collections.Counter(c['test'] for c in calls) == original_names(), 'changed original subtest inventory'
    assert {test:sum(c['test'].split('/')[0] == test for c in calls) for test in TESTS} == TESTS
    for call in calls:
        assert set(call) == {'test','call','input','result'}
        if SUITE == 'svg-links':
            assert set(call['input']) == {'operation','board','key','source_root','paths','render'}, 'expected result in native request'
            assert call['input']['operation'] == 'svg-links'
            assert set(call['result']) == {'board','input_board','error','svg','render_error'}
        elif SUITE == 'paged-export' and call['input'].get('operation') == 'rebase':
            assert set(call['input']) == {'operation','links','source_root','pages'}, 'expected result in native request'
            assert set(call['result']) == {'links','input_links','error'}
        elif SUITE == 'image' and 'scale_bits' in call['input']:
            assert set(call['input']) == {'format','scale_bits'}, 'expected result in native request'
            assert set(call['result']) == {'format','width','height','error'}
        else:
            assert set(call['input']) == {'directory','args','env','canceled','files'}, 'expected result in native request'
            assert set(call['result']) == {'error','stdout','files'}
    rows = []
    for test in TESTS:
        indices = [i for i,c in enumerate(calls) if c['test'].split('/')[0] == test]
        matches = sum(len(results)==len(calls) and
            (svg_link_comparison(calls[i], results[i]) if SUITE == 'svg-links'
             else paged_export_comparison(calls[i], results[i]) if SUITE == 'paged-export'
             else surface_icon_comparison(calls[i], results[i]) if SUITE == 'surface-icons'
             else image_export_comparison(calls[i], results[i]) if SUITE in {'image-export', 'source-gif'}
             else svg_export_comparison(calls[i], results[i]) if SUITE in {'svg-export', 'svg-publication'}
             else calls[i]['result']==results[i]) for i in indices)
        if SUITE == 'source-gif' and matches == len(indices):
            # Preserve the original cross-entry-point byte equality assertion,
            # without requiring Go and Rust's GIF encoders to use the same bytes.
            if results[indices[0]]['stdout'] != results[indices[1]]['stdout']:
                matches = 0
        rows.append(dict(test=test, calls=len(indices), matched_calls=matches, outcome='match' if matches==len(indices) else 'mismatch'))
    return rows

def svg_link_comparison(call, actual):
    """All rebased/input target fields and SVG bytes remain exact."""
    if not LANGUAGE.same_json(call['result'], actual) or not LANGUAGE.same_json(actual['input_board'], call['input']['board']):
        return False
    if call['input']['render'] is None:
        return actual['svg'] is None and not actual['render_error']
    if actual['error'] or actual['render_error']:
        return False
    try:
        root = ET.fromstring(actual['svg'])
        return (root.tag == '{http://www.w3.org/2000/svg}svg' and
                any(node.tag in {'{http://www.w3.org/2000/svg}path',
                                 '{http://www.w3.org/2000/svg}polygon'} for node in root.iter()))
    except (TypeError, ValueError, ET.ParseError):
        return False

def surface_icon_comparison(call, actual):
    # Each original asset is rendered both as PNG and as SVG. Keep the
    # complete raster inventory gate and exact vector bytes for both paths.
    if '--stdout-format=png' in call['input']['args']:
        return image_export_comparison(call, actual)
    return svg_export_comparison(call, actual)

def worker(runner, request, response, native_test=None):
    results = []
    calls = json.loads(request.read_text())
    if SUITE == 'svg-links':
        assert native_test is not None
        environment = dict(os.environ,D2RS_SVG_LINK_REQUEST=str(request),D2RS_SVG_LINK_RESPONSE=str(response))
        with (request.parent/'svg-links.log').open('wb') as log:
            subprocess.run([str(native_test),'--ignored','--exact','isometric_cli::svg_api_tests::replay','--test-threads=1'],env=environment,stdout=log,stderr=log,check=True,timeout=120)
        assert len(json.loads(response.read_text())) == len(calls), 'missing/extra native SVG link results'
        return
    rebases = [c for c in calls if c.get('operation') == 'rebase']
    rebase_results = iter([])
    if rebases:
        assert native_test is not None
        rebase_request, rebase_response = request.parent/'rebase-input.json', request.parent/'rebase-output.json'
        rebase_request.write_text(json.dumps(rebases)+'\n')
        environment = dict(os.environ,D2RS_PAGED_REQUEST=str(rebase_request),D2RS_PAGED_RESPONSE=str(rebase_response))
        with (request.parent/'rebase.log').open('wb') as log:
            subprocess.run([str(native_test),'--ignored','--exact','isometric_cli::paged::api_tests::replay','--test-threads=1'],env=environment,stdout=log,stderr=log,check=True,timeout=120)
        values = json.loads(rebase_response.read_text())
        assert len(values) == len(rebases), 'missing/extra native rebase results'
        rebase_results = iter(values)
    options = [c for c in calls if 'scale_bits' in c]
    option_results = iter([])
    if options:
        assert native_test is not None
        option_request, option_response = request.parent/'options-input.json', request.parent/'options-output.json'
        option_request.write_text(json.dumps(options)+'\n')
        environment = dict(os.environ,D2RS_IMAGE_REQUEST=str(option_request),D2RS_IMAGE_RESPONSE=str(option_response))
        with (request.parent/'options.log').open('wb') as log:
            subprocess.run([str(native_test),'--ignored','--exact','isometric_cli::image_api_tests::replay','--test-threads=1'],env=environment,stdout=log,stderr=log,check=True,timeout=120)
        values = json.loads(option_response.read_text())
        assert len(values) == len(options), 'missing/extra native option results'
        option_results = iter(values)
    for i,call in enumerate(calls):
        if call.get('operation') == 'rebase':
            results.append(next(rebase_results));continue
        if 'scale_bits' in call:
            results.append(next(option_results));continue
        directory = Path(call['directory']); assert directory.is_dir()
        assert directory.resolve().is_relative_to(request.parent.parent/'work')
        assert not call['canceled']
        # These paths belong solely to the captured Go test. The complete old
        # outputs remain in the immutable call records before native replay.
        for path in directory.rglob('*'):
            assert not path.is_symlink(), 'replay cannot write through an alias'
            if path.is_file(): path.unlink()
        for name,file in call['files'].items():
            if file is not None:
                path = directory/name; path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(base64.b64decode(file['data'])); path.chmod(file['mode'])
        environment = dict(item.split('=',1) for item in call['env'])
        stdout, stderr = request.parent/f'{i:03d}.stdout', request.parent/f'{i:03d}.stderr'
        # Help includes argv[0]. Use the same program name as xmain.TestState.
        executable = runner
        if SUITE == 'network':
            executable = request.parent/'d2'
            if not executable.exists(): executable.symlink_to(runner)
        with stdout.open('wb') as out, stderr.open('wb') as err:
            process = subprocess.run([str(executable),*call['args']],cwd=directory,env=environment,stdout=out,stderr=err,timeout=120)
        assert (process.returncode != 0) == bool(stderr.read_bytes().startswith(b'err:')), 'unexpected native exit/terminal error'
        results.append(dict(error=stderr.read_text() if process.returncode else '', stdout=base64.b64encode(stdout.read_bytes()).decode(),files=file_state(directory)))
    response.write_text(json.dumps(results)+'\n')

def main():
    if len(sys.argv)>1 and sys.argv[1]=='--worker':
        configure(sys.argv[5])
        worker(Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4]),Path(sys.argv[6]) if len(sys.argv)>6 else None); return 0
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust',required=True,type=Path);parser.add_argument('--capture',required=True,type=Path)
    parser.add_argument('--output',required=True,type=Path);parser.add_argument('--reuse',action='store_true')
    parser.add_argument('--suite',choices=['isometric','export','network','image','image-export','svg-export','svg-publication','paged-export','surface-icons','source-gif','svg-links'],default='isometric')
    parser.add_argument('--native-test',type=Path)
    args=parser.parse_args();configure(args.suite)
    source=references.verify_source();capture=args.capture.resolve();runner=args.rust.resolve();digest=sha(runner)
    spec=importlib.util.spec_from_file_location('cli',ROOT/'scripts/cli-scoreboard.py');cli=importlib.util.module_from_spec(spec);spec.loader.exec_module(cli)
    hook=ROOT/'tools/cli-mode-scoreboard/hook_test.go'
    names = ['isometric_test.go'] if SUITE in {'isometric', 'source-gif'} else ['export_test.go','isometric_image_test.go']
    if SUITE == 'network': names = ['help_test.go']
    if SUITE in {'image', 'image-export'}: names = ['isometric_image_test.go']
    if SUITE == 'svg-export': names = ['isometric_image_test.go', 'isometric_svg_test.go']
    if SUITE == 'svg-publication': names = ['isometric_image_test.go', 'isometric_integration_test.go']
    if SUITE == 'paged-export': names = ['isometric_image_test.go', 'isometric_paged_test.go']
    if SUITE == 'svg-links': names = ['isometric_svg_test.go']
    if SUITE == 'surface-icons': names = ['isometric_image_test.go', 'isometric_surface_test.go']
    originals = {name:(source/'d2cli'/name).read_bytes() for name in names}
    extra = ROOT/'tools/cli-mode-scoreboard/export_hook_test.go'
    if SUITE == 'network': extra = ROOT/'tools/cli-mode-scoreboard/network_hook_test.go'
    if SUITE in {'image', 'image-export', 'svg-export', 'svg-publication'}: extra = ROOT/'tools/cli-mode-scoreboard/image_hook_test.go'
    if SUITE == 'paged-export': extra = ROOT/'tools/cli-mode-scoreboard/paged_hook_test.go'
    if SUITE == 'svg-links': extra = ROOT/'tools/cli-mode-scoreboard/svg_links_hook_test.go'
    if SUITE == 'surface-icons': extra = ROOT/'tools/cli-mode-scoreboard/image_hook_test.go'
    version_flags = '-X github.com/d2lang/d2/lib/version.Version=v0.9.0'
    bindings=dict(source_commit=references.SOURCE_COMMIT,package=PACKAGE,hook_sha256=sha(hook),sources={'d2cli/'+name:hashlib.sha256(data).hexdigest() for name,data in originals.items()},version_profile=dict(go_ldflags=version_flags,rust='release v0.9.0; no output normalization'))
    if SUITE != 'isometric': bindings.update(suite=SUITE)
    if SUITE not in {'isometric', 'source-gif'}: bindings.update(extra_hook_sha256=sha(extra))
    if SUITE == 'paged-export':
        bindings['pdf_clock'] = dict(epoch_seconds=946782245,timezone='UTC',catalog_order='FPDF SetCatalogSort(true)',
            oracle_environment={'SOURCE_DATE_EPOCH':'946782245'},
            sources={name:sha(source/name) for name in ['lib/pdf/pdf.go']},
            helpers={name:sha(ROOT/'tools/cli-scoreboard'/name) for name in ['pdf_clock.go','pdf_clock_test.go']})
    native_test = args.native_test.resolve() if args.native_test else None
    if SUITE in {'image','paged-export','svg-links'}: assert native_test is not None
    native_test_hash = sha(native_test) if native_test else None
    env=dict(os.environ,GOMODCACHE=str(ROOT/'.reference/go-mod/pkg/mod'),GOCACHE=str(ROOT/'.reference/go-build'),GOFLAGS='-mod=mod',GOPROXY='off',TMPDIR=str(capture/'tmp'),D2RS_MODE_CAPTURE=str(capture))
    if SUITE == 'paged-export': env['SOURCE_DATE_EPOCH'] = '946782245'
    if not args.reuse:
        capture.mkdir(exist_ok=False);(capture/'calls').mkdir();(capture/'tmp').mkdir()
        directory=references.TRACE_CHECKOUT/'d2cli';helper=directory/'d2rs_cli_mode_test.go';extra_target=directory/'d2rs_cli_export_test.go'
        assert all((directory/name).read_bytes()==data for name,data in originals.items()) and not helper.exists() and not extra_target.exists(), 'concurrent source mutation'
        try:
            for name,data in originals.items():
                text=data.decode()
                if SUITE in {'isometric', 'source-gif'}:
                    assert text.count('dir := t.TempDir()')==1 and text.count('Run: Run,')==1
                    text=text.replace('dir := t.TempDir()','dir := d2rsModeDir(t)')
                    text=text.replace('Run: Run,','Run: func(ctx context.Context, ms *xmain.State) error { return d2rsModeRun(t, ctx, ms, dir, append([]string{input}, args...)) },')
                elif name == 'export_test.go':
                    assert text.count('t.TempDir()') == 2
                    text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                elif SUITE == 'export':
                    old='Run: Run, Args: append([]string{"d2", input}, args...)'
                    assert text.count(old)==1
                    text=text.replace(old,'Run: func(ctx context.Context, ms *xmain.State) error { return d2rsModeRun(t, ctx, ms, dir, append([]string{input}, args...)) }, Args: append([]string{"d2", input}, args...)')
                elif SUITE == 'network':
                    assert text.count('Run:    Run,') == 1
                    text=text.replace('Run:    Run,','Run: func(ctx context.Context, ms *xmain.State) error { return d2rsModeRun(t, ctx, ms, d2rsModeDir(t), []string{"--help"}) },')
                    old='privateNetworkEnvDefault(&xmain.State{Env:'
                    assert text.count(old) == 1
                    text=text.replace(old,'d2rsNetworkDefault(t, &xmain.State{Env:')
                elif SUITE == 'svg-links':
                    assert text.count('relinkIsometricSVGBoard(') == 2
                    assert text.count('d2isometricimg.Render(') == 1
                    text=text.replace('relinkIsometricSVGBoard(', 'd2rsSVGLinkRebase(t, ')
                    text=text.replace('d2isometricimg.Render(', 'd2rsSVGLinkRender(t, ')
                elif SUITE == 'svg-export' and name == 'isometric_svg_test.go':
                    assert text.count('t.TempDir()') == 7
                    text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                elif SUITE == 'svg-publication' and name == 'isometric_integration_test.go':
                    assert text.count('t.TempDir()') == 4
                    text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                elif SUITE == 'paged-export' and name == 'isometric_paged_test.go':
                    assert text.count('t.TempDir()') == 6 and text.count('rebaseIsometricPageLinks(') == 2
                    text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                    text=text.replace('rebaseIsometricPageLinks(', 'd2rsPagedRebase(t, ')
                elif SUITE == 'surface-icons' and name == 'isometric_surface_test.go':
                    assert text.count('t.TempDir()') == 1
                    text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                else:
                    old='Run: Run, Args: append([]string{"d2", input}, args...)'
                    assert text.count(old)==1
                    text=text.replace(old,'Run: func(ctx context.Context, ms *xmain.State) error { return d2rsModeRun(t, ctx, ms, dir, append([]string{input}, args...)) }, Args: append([]string{"d2", input}, args...)')
                    if SUITE not in {'paged-export', 'surface-icons'}:
                        assert text.count('isometricImageOptions(')==2
                        text=text.replace('isometricImageOptions(', 'd2rsImageOptions(t, ')
                    elif SUITE == 'paged-export':
                        text=text.replace('[]string{"PATH="}', '[]string{"PATH=", "SOURCE_DATE_EPOCH=946782245"}')
                    text=text.replace('dir := t.TempDir()', 'dir := d2rsModeDir(t)')
                    if SUITE == 'image-export':
                        text=text.replace('t.TempDir()', 'd2rsModeDir(t)')
                (directory/name).write_text(text)
            helper.write_bytes(hook.read_bytes())
            if SUITE not in {'isometric', 'source-gif'}: extra_target.write_bytes(extra.read_bytes())
            with paged_clock():
                go=cli.bounded_run(['go','test','-ldflags='+version_flags,'./d2cli','-run','^('+'|'.join(TESTS)+')$','-json','-count=1','-parallel=1'],env,capture,capture/'go.jsonl',600)
        finally:
            for name,data in originals.items(): (directory/name).write_bytes(data)
            helper.unlink(missing_ok=True);extra_target.unlink(missing_ok=True)
        assert go=={'exit':0,'failure':None},go
        events=cli.events(capture/'go.jsonl');assert all(events.get(t)=='pass' for t in TESTS),events
        paths=sorted((capture/'calls').glob('*.json'))+[capture/'go.jsonl']
        (capture/'manifest.json').write_text(json.dumps(dict(bindings,go=go,original_assertions=events,captured_sha256={str(p.relative_to(capture)):sha(p) for p in paths}),indent=2)+'\n')
    manifest=json.loads((capture/'manifest.json').read_text());assert all(manifest[k]==v for k,v in bindings.items()),'changed source/hook'
    assert manifest['go']=={'exit':0,'failure':None}
    paths=sorted((capture/'calls').glob('*.json'))+[capture/'go.jsonl'];assert {str(p.relative_to(capture)):sha(p) for p in paths}==manifest['captured_sha256'],'changed capture bytes/inventory'
    calls=[json.loads(p.read_text()) for p in paths if p.suffix=='.json'];compare(calls,[])
    assert all(manifest['original_assertions'].get(c['test'])=='pass' for c in calls)
    replay=capture/('replay-'+args.output.stem);replay.mkdir(exist_ok=False);request,response=replay/'input.json',replay/'output.json'
    request.write_text(json.dumps([c['input'] for c in calls])+'\n')
    command=[sys.executable,str(Path(__file__).resolve()),'--worker',str(runner),str(request),str(response),SUITE]
    if native_test: command.append(str(native_test))
    native=cli.bounded_run(command,env,replay,replay/'native.log',600,extra_directories=(capture/'work',))
    results=json.loads(response.read_text()) if native=={'exit':0,'failure':None} and response.is_file() else []
    assert sha(runner)==digest,'runner changed';rows=compare(calls,results)
    report=dict(bindings,runner=str(runner),runner_sha256=digest,capture=str(capture),replay=str(replay),rows=rows,calls=calls,results=results,native=native,original_assertions=manifest['original_assertions'],controls=dict(total_bytes=1<<30,file_bytes=128<<20,minimum_free_bytes=20<<30,deadline_seconds=600),projection='Original CLI args/environment and source bytes; complete stdout bytes, terminal errors and complete input/output file inventory/bytes/modes. Original Go assertions unchanged. No log normalization.')
    if SUITE in {'image-export', 'source-gif'}:
        report['projection'] = 'Original Go assertions unchanged. Exact complete errors, file inventory, modes and non-image bytes. Every PNG passes shared complete-output/manifest gate plus exact RGBA samples. GIF compares all composited RGBA frames, delays, canvas and loop. Encoded image byte identity is not required.'
    if SUITE in {'svg-export', 'svg-publication'}:
        report['projection'] += ' Every successful emitted SVG must parse with the SVG namespace and contain vector geometry; no SVG byte normalization.'
    if SUITE == 'paged-export':
        report['projection'] += ' Full PDF/PPTX bytes under explicit shared UTC clock/catalog sort and release version; document page/image/ZIP/XML validation. PNG uses the complete raster/manifest gate. Direct link rebase calls observe target/URL and unchanged input; box/tooltip are opaque preserved metadata through the production packaging adapter.'
    if SUITE == 'svg-links':
        report['projection'] = 'Original relink and Render calls with unchanged Go assertions. Complete rebased target JSON, original target preservation, complete errors and raw SVG bytes; no normalization. Native test adapter translates local Go path-map keys into the production Rust full-source keys. Root-link mutation, allocated empty-slice identity and error cases are not exercised by these original inputs.'
    if native_test:
        assert sha(native_test)==native_test_hash,'test runner changed'
        report.update(native_test=str(native_test),native_test_sha256=native_test_hash)
    if SUITE == 'image': report['projection'] += ' Direct image-options calls compare format, width, height (including error returns) and complete error. Other Options fields are not qualified.'
    if SUITE == 'surface-icons': report['projection'] += ' PNG uses the shared complete-output gate and exact RGBA samples; SVG bytes are exact and every SVG must parse with vector geometry. Both local SVG and PNG assets run with PATH empty. All original artwork and embedded-raster assertions remain active.'
    if SUITE == 'source-gif': report['projection'] += ' Native source-selected and flag-selected GIFs must also be byte-identical to each other, preserving the original cross-entry-point equality assertion.'
    args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(rows,indent=2))
    return int(any(r['outcome']!='match' for r in rows))

if __name__=='__main__':raise SystemExit(main())
