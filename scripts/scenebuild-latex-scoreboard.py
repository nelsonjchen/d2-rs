#!/usr/bin/env python3
"""Complete scenes/errors from every original master latex_test.go Build call."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "scenebuild_assets", Path(__file__).with_name("scenebuild-asset-scoreboard.py")
)
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)
assets.TEST_FILE = "latex_test.go"
assets.TESTS = {
    "TestBuildLatexShapeAndConnectionLabels": 1,
    "TestBuildLatexUsesIntrinsicViewportAndPlacement": 1,
    "TestBuildLatexResolvesCurrentColorWithoutOverridingExplicitColor": 1,
    "TestBuildLatexInputAndImportBudgets": 3,
    "TestBuildLatexConnectionMask": 1,
    "TestBuildLatexLabelRendersPixels": 1,
}
assets.UNCOVERED = []
if __name__ == "__main__":
    assets.main()
