#!/usr/bin/env python3
"""Original remaining route, opacity, background and shadow contracts.

Original assertions remain active. 1 GiB capture / 128 MiB file / 20 GiB free,
600s Go and 120s per native function. Input-only requests, complete pixel gate.
"""
import importlib.util
from pathlib import Path
spec=importlib.util.spec_from_file_location('composition',Path(__file__).with_name('isometric-composition-scoreboard.py'))
composition=importlib.util.module_from_spec(spec);spec.loader.exec_module(composition)
ROOT=Path(__file__).resolve().parents[1]
TESTS={
 'TestFilledArrowStemsDoNotBluntTheTip': {'edges':5},
 'TestContainerBackgroundPreservesSourceAndThemePaint': {'native':6},
 'TestNativeGradientBackgroundAnimationAndAlpha': {'native':1,'native-frame':2},
 'TestHierarchyFixtureGroundShadowIgnoresUnrelatedDepth': {'native':1,'render':2},
 'TestHierarchyAnimationKeepsBothShadowCameras': {'native':1,'native-frame':3},
 'TestOpacityGroupFreezeAdmissionAndWork': {'snapshot':1,'validate':4,'draw':3},
 'TestOpacityGroupShadowAndDeferredSnapshot': {'shadow':2,'render':3},
 'TestDistantOpacityGroupKeepsOrdinaryCapAndLabel': {'render':2,'snapshot':1},
 'TestProjectedCaptionReservesLaterAuthoredAnchor': {'edges':1},
 'TestNativeOnEdgeCaptionMasksWireWithResolvedThemeFill': {'edges':1,'render':2},
 'TestNativePacketsRequireAuthoredAnimation': {'edges':3},
 'TestNativeConnectionCaptionFillAndOpacity': {'edges':1},
 'TestNativeRouteCaptionsKeepEndpointAndIconPrintSpace': {'edges':2},
 'TestNativeEdgeIconOnlyAndTextCaption': {'edges':2},
 'TestSequenceBackgroundPreservesSourceAndThemePaint': {'native':4},
 'TestQueueContactArrowsTrafficAndMainCaptionUseCorrectAnchors': {'edges':1},
 'TestUprightContactArrowsTrafficAndMainCaptionUseCorrectAnchors': {'edges':4},
 'TestSolidCapShadowKeepsPhysicalFillOpacity': {'node':4,'shadow':8},
 'TestShadowTextureHolesAndPartialCoverageRemain': {'shadow':1},
 'TestNativeSVGCoveragePreservesPhysicalGeometry': {'native':2},
}
FILES=['arrow_visibility_test.go','container_background_test.go','gradient_paints_test.go','ground_receivers_integration_test.go','opacity_groups_test.go','projection_clearance_test.go','route_caption_style_test.go','route_lanes_test.go','sequence_background_test.go','solid_contacts_test.go','solid_shadow_test.go','svg_test.go']
def scoreboard():
 score=composition.base(
  tests={n:sum(ops.values()) for n,ops in TESTS.items()}, files=FILES,
  sources=['geometry.go','render.go','raster.go','opacity_groups.go','route_caption_style.go','solid_contacts.go','ground_receivers.go','route_caption_mask.go'],
  template=ROOT/'tools/isometric-residual-scoreboard/hook_test.go',
  helper_templates=[ROOT/'tools/isometric-solid-scoreboard/hook_test.go',ROOT/'tools/isometric-shape-scoreboard/hook_test.go',ROOT/'tools/isometric-hierarchy-scoreboard/hook_test.go',ROOT/'tools/isometric-composition-scoreboard/hook_test.go',ROOT/'tools/isometric-residual-scoreboard/raster_test.go'],
  helper_functions=['testSolidContactArrowsTrafficAndMainCaption'],
  call_replacements=[
   ('testSolidContactArrowsTrafficAndMainCaption(t,', 'residualContactTest(t,'),
   ('b.edges(', 'residualEdges(b, '), ('b.node(', 'compositionNode(b, "node", '),
   ('d2isometric.BuildScene(', 'residualScene(d2isometric.BuildScene, '),
   ('newNativeSceneWithOptions(', 'residualNative('), ('newNativeScene(', 'residualNativeDefault('),
   ('native.Frame(', 'residualNativeFrame(native, '), ('scene.frameRaster(', 'residualFrameRaster(scene, '),
   ('rasterSnapshot(', 'rasterObserveSnapshot('), ('rasterValidate(', 'residualValidate('), ('rasterBuildShadow(', 'residualShadow('),
   ('newRaster(', 'rasterObserveConstruct('), ('NewRaster(', 'rasterObserveNew('),
   ('raster.Frame(', 'rasterObserveFrame(raster, '), ('r.Frame(', 'rasterObserveFrame(r, '),
   ('r.rasterPrepare(', 'residualPrepare(r, '), ('r.drawPrepared(', 'residualDraw(r, '),
  ])
 original_capture=score.capture_original
 def capture_original(capture):
  path=score.references.TRACE_CHECKOUT/'d2renderers/d2isometric/d2isometricimg/route_caption_mask.go'
  pristine=score.references.verify_source()/'d2renderers/d2isometric/d2isometricimg/route_caption_mask.go'
  data=path.read_bytes();assert data==pristine.read_bytes()
  replacement=data.replace(b'func (b *meshBuilder) routeCaptionKnockout(',b'func (b *meshBuilder) d2rsOriginalCaptionKnockout(',1);assert replacement!=data
  try:
   path.write_bytes(replacement);original_capture(capture)
  finally:path.write_bytes(data)
 score.capture_original=capture_original
 score.OPS=TESTS
 score.RUNNER_TEST='img::native_edges::residual_api_tests::replay_original_calls'
 score.PIXEL_OPS={"render","draw","native-frame"}
 score=composition.pixel_score(score)
 score.COMPARISON_CONTRACT={
  'original_go_assertions':'all retained and required to pass',
  'compared':['ordered meshes/material, opacity-group and owner sharing; complete packet routes; source edges and scene/target values','native scene cameras and backgrounds','full nonempty raster inventories and complete RGBA bytes by SHA-256','complete shadow camera, depth and opacity bytes by SHA-256, bias and work remaining','snapshot ownership and validation errors; prepared-draw errors and complete destination pixels'],
  'unqualified':['Go allocation, cached scratch-buffer layout and pointer addresses','original caller-only assertions outside observed operations','native frames are rebuilt from their original source target; Go cached-raster reuse is not claimed','prepared-draw scratch size and exact internal work remaining are Go implementation details, not compared']}
 # Both rendering operation kinds emit complete raw pixel inventories.
 original_hashes=score.hashes
 def hashes(capture):
  return {**original_hashes(capture),**{str(p.relative_to(capture)):score.sha(p) for p in sorted([*(capture/"inputs").glob("*.bin"),*(capture/"diagnostic").glob("*.json")])}}
 score.hashes=hashes
 original_verify=score.verify
 def verify(capture):
  cases=original_verify(capture)
  inputs=set()
  def walk(v):
   if isinstance(v,dict):
    if "pixels_file" in v:
     p=Path(v["pixels_file"]);assert p.parent==capture/"inputs" and p.is_file() and not p.is_symlink()
     assert p.name==score.sha(p)+".bin";inputs.add(p)
    for child in v.values():walk(child)
   elif isinstance(v,list):
    for child in v:walk(child)
  walk(cases)
  assert inputs==set((capture/"inputs").glob("*.bin"))
  return cases
 score.verify=verify
 return score
if __name__=='__main__': composition.main(scoreboard)
