#!/usr/bin/env python3
"""Original solid contacts, projection clearance and sidewall geometry calls."""

import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "geometry", Path(__file__).with_name("isometric-geometry-scoreboard.py")
)
geometry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(geometry)
base = geometry.scoreboard
TESTS = {
    "TestQueueContactsReachActualBarrelAndSharpCaps": {"queue": 5},
    "TestQueueContactRoutesPreserveSourceVerticesAndBounds": {
        "contacts": 2,
        "queue": 3,
    },
    "TestQueueContactsMatchFlattenedPrintedCrown": {"queue": 6, "crown": 1},
    "TestUprightContactsReachActualSharpRimsAndWalls": {"upright": 120},
    "TestCylinderContactClosesRealRingBufferGaps": {"contacts": 2},
    "TestUprightContactsDoNotInventExteriorRoutes": {"upright": 6},
    "TestSolidContactsFollowLowerSupportsAtOriginalRouteHeight": {
        "contacts": 12,
        "crown": 3,
    },
    "TestSolidContactSupportDefaultsPreserveExistingRoutes": {"contacts": 6},
    "TestEdgeRoutesApplySupportOnlyToOrdinarySolidContacts": {"pipeline": 1},
    "TestProjectedCaptionClearanceAcrossElevations": {"place": 3, "avoid": 2},
    "TestProjectedCaptionMeshIncludesFinalOutsideSurfaces": {
        "place": 2,
        "avoidMesh": 1,
        "score": 1,
    },
    "TestProjectedCaptionAvoidsLaterCrossingWire": {"place": 2, "avoidRoute": 1},
    "TestProjectedCaptionMeshKeepsClearSilhouetteCorners": {
        "avoidMesh": 1,
        "score": 2,
        "place": 1,
    },
    "TestRouteCasingClearsTerracesWithoutMovingInkOrBridge": {"ink": 12, "casing": 12},
    "TestHierarchyCasingFloorOnlyUsesVisibleRaisedCaps": {"floor": 4},
    "TestNativeRoundedSidewallsSmoothWithoutMovingContour": {
        "faceSource": 1,
        "profile": 1,
        "extrude": 2,
    },
    "TestNativeSidewallsKeepSharpAndConcaveCorners": {"extrude": 2},
    "TestNativeFaceClearsNonpaintInteractionMetadata": {"faceSource": 1},
    "TestClassicPalettePreservesAuthoredAndThemeOverrideInk": {"classic": 3},
    "TestClassicPalettePreservesStructuredDocumentBackgrounds": {"classic": 2},
}


def scoreboard():
    score = base(
        tests={n: sum(ops.values()) for n, ops in TESTS.items()},
        files=[
            "solid_contacts_test.go",
            "solid_contacts_support_test.go",
            "projection_clearance_test.go",
            "route_casing_clearance_test.go",
            "native_face_ink_test.go",
            "native_shapes_test.go",
            "native_page_test.go",
        ],
        sources=[
            "solid_contacts.go",
            "geometry_routes.go",
            "route_labels.go",
            "projection_clearance.go",
            "route_style.go",
            "hierarchy_boards.go",
            "hierarchy_support.go",
            "native_shapes.go",
            "native_style.go",
        ],
        template=Path(__file__).resolve().parents[1]
        / "tools/isometric-contact-scoreboard/hook_test.go",
        call_replacements=[
            ("nativeQueueContact(", "d2rsContactQueue("),
            ("nativeUprightContact(", "d2rsContactUpright("),
            ("nativeQueueCrown(", "d2rsContactCrown("),
            ("nativeSolidContactRoutes(", "d2rsContactRoutes("),
            ("nativeEdgeRoutes(", "d2rsContactPipeline("),
            ("newRouteCaptionPlacer()", "d2rsContactPlacer()"),
            ("p.score(", "d2rsContactScore(p, "),
            ("hierarchyCasingFloor(", "d2rsContactFloor("),
            ("base.routeInk(", "d2rsContactInk(base, "),
            ("raised.routeInk(", "d2rsContactInk(raised, "),
            ("base.routeCasing(", "d2rsContactCasing(base, "),
            ("raised.routeCasing(", "d2rsContactCasing(raised, "),
            ("nativeFaceSource(", "d2rsContactFaceSource("),
            ("nativeShapeProfile(", "d2rsContactProfile("),
            ("b.extrudedProfile(", "d2rsContactExtrude(b, "),
            ("nativeClassicNode(", "d2rsContactClassic("),
        ],
    )
    score.OPS = TESTS
    score.RUNNER_TEST = "img::route_labels::contact_api_tests::replay_original_calls"
    score.COMPARISON_CONTRACT = {
        "compared": [
            "complete contact points, paths, lanes and errors",
            "complete ordered obstacle state and work counts",
            "full generated route and sidewall vertices/normals/materials",
            "complete native source and palette nodes, profiles and source immutability",
        ],
        "original_go_assertions": "all retained and required to pass",
        "unqualified": [
            "unobserved callers and raster assertions",
            "Go allocation, map layout and pointer addresses",
        ],
        "inputs": "projection mesh materials preserve sharing/alpha/texture presence, without unused texture bytes; nonfinite point coordinates retain classification",
    }
    return score


if __name__ == "__main__":
    geometry.scoreboard = scoreboard
    geometry.main()
