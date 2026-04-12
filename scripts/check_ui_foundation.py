#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
QML_ROOT = ROOT / "pier-ui-qt" / "qml"

ALLOWED_RAW = {
    QML_ROOT / "Main.qml",
    QML_ROOT / "components" / "PopoverPanel.qml",
    QML_ROOT / "components" / "PierComboBox.qml",
    QML_ROOT / "components" / "PierScrollView.qml",
    QML_ROOT / "components" / "PierScrollBar.qml",
    QML_ROOT / "components" / "PierSlider.qml",
    QML_ROOT / "components" / "PierTextArea.qml",
    QML_ROOT / "components" / "PierTextField.qml",
    QML_ROOT / "components" / "PierSearchField.qml",
}

RULES = [
    (re.compile(r"\bPopup\s*\{"), "Use ModalDialogShell or PopoverPanel instead of raw Popup."),
    (re.compile(r"\bTextField\s*\{"), "Use PierTextField or PierSearchField instead of raw TextField."),
    (re.compile(r"\bTextArea\s*\{"), "Use PierTextArea instead of raw TextArea."),
    (re.compile(r"\bTextInput\s*\{"), "Use PierTextField, PierTextArea, or PierSearchField instead of raw TextInput."),
    (re.compile(r"\bScrollView\s*\{"), "Use PierScrollView instead of raw ScrollView."),
    (re.compile(r"ScrollBar\.(?:vertical|horizontal):\s*ScrollBar\b"), "Use PierScrollBar instead of raw ScrollBar."),
    (re.compile(r"\bSlider\s*\{"), "Use PierSlider instead of raw Slider."),
    (re.compile(r"\bMenuItem\s*\{"), "Use PierMenuItem for in-app menus; native app menu bar in Main.qml is the only exception."),
    (re.compile(r"\bMenu\s*\{"), "Use PopoverPanel for in-app floating menus; native app menu bar in Main.qml is the only exception."),
]


def should_check(path: Path) -> bool:
    if path in ALLOWED_RAW:
        return False
    return path.suffix == ".qml"


def main() -> int:
    failures: list[str] = []
    for path in sorted(QML_ROOT.rglob("*.qml")):
        if not should_check(path):
            continue
        text = path.read_text(encoding="utf-8")
        for pattern, message in RULES:
            for match in pattern.finditer(text):
                line = text.count("\n", 0, match.start()) + 1
                failures.append(f"{path}:{line}: {message}")

    if failures:
        print("Pier-X UI foundation violations found:", file=sys.stderr)
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1

    print("Pier-X UI foundation check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
