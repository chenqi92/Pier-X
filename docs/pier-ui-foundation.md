# Pier-X UI Foundation

Pier-X must be built on a small, strict set of reusable QML controls. Feature pages should not compose raw Qt controls with one-off styling.

## Single source of truth

- Tokens: [Theme.qml](/Users/chenqi/Projects/workspace-freq/Pier-X/pier-ui-qt/qml/Theme.qml)
- Design rules: [SKILL.md](/Users/chenqi/Projects/workspace-freq/Pier-X/.agents/skills/pier-design-system/SKILL.md)
- Agent rules: [AGENTS.md](/Users/chenqi/Projects/workspace-freq/Pier-X/AGENTS.md)

## Approved foundation controls

- Buttons
  - `PrimaryButton`
  - `GhostButton`
  - `IconButton`
- Inputs
  - `PierTextField`
  - `PierTextArea`
  - `PierSearchField`
  - `PierComboBox`
  - `PierScrollView`
  - `ToggleSwitch`
  - `PierSlider`
- Surfaces
  - `Card`
  - `ToolPanelSurface`
  - `ModalDialogShell`
  - `PopoverPanel`
- Utility
  - `SegmentedControl`
  - `StatusPill`
  - `PierToolTip`
  - `PierMenuItem`
  - `PierScrollBar`

## Hard rules

1. Do not instantiate raw `Popup` in feature pages.
2. Do not instantiate raw `TextField` in feature pages.
3. Do not instantiate raw `TextArea` in feature pages.
4. Do not instantiate raw `TextInput` in feature pages.
5. Do not instantiate raw `ScrollView` in feature pages.
6. Do not instantiate raw `ScrollBar` in feature pages.
7. Do not hand-style menu rows inside pages.
8. If a feature needs a new primitive, add it to `qml/components/` first.

## Allowed exceptions

- Native app menu bar entries may use `MenuBar` and `MenuItem`.
- Foundation wrappers may internally use raw Qt controls where required:
  - `PopoverPanel.qml`
  - `PierComboBox.qml`
  - `PierTextArea.qml`
  - `PierTextField.qml`
  - `PierSearchField.qml`
  - `PierScrollView.qml`
  - `PierSlider.qml`
  - `PierScrollBar.qml`

## Migration target

Every page should converge toward:

- one surface language
- one field language
- one popover language
- one menu language
- one scrollbar language

## Audit

Run:

`python3 /Users/chenqi/Projects/workspace-freq/Pier-X/scripts/check_ui_foundation.py`

This flags raw control usage outside approved wrapper files.
