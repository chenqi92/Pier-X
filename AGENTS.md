# Pier-X Agent Rules

This repository uses a strict UI design system.

## Mandatory skill

- For any QML or UI-related work, always apply:
  `/Users/chenqi/Projects/workspace-freq/Pier-X/.agents/skills/pier-design-system/SKILL.md`

## Foundation controls

- Do not introduce raw `Popup` in feature pages. Use:
  - `ModalDialogShell`
  - `PopoverPanel`
- Do not introduce raw `TextField` in feature pages. Use:
  - `PierTextField`
  - `PierTextArea`
  - `PierSearchField`
- Do not introduce raw `TextArea` in feature pages. Use `PierTextArea`.
- Do not introduce raw `TextInput` in feature pages. Use `PierTextField`, `PierTextArea`, or `PierSearchField`.
- Do not introduce raw `ScrollView` in feature pages. Use `PierScrollView`.
- Do not introduce raw `ScrollBar` in feature pages. Use `PierScrollBar`.
- Prefer `PierScrollView` over raw `ScrollView` in feature pages.
- Do not duplicate page-local menu row visuals. Use `PierMenuItem`.
- Do not duplicate slider visuals. Use `PierSlider`.

## Exceptions

- Native application menu bar entries in `Main.qml` may use `MenuBar` and `MenuItem`.
- `Popup` is allowed only inside foundation wrappers such as:
  - `PopoverPanel.qml`
  - `PierComboBox.qml`
  - `PierTextArea.qml`
  - `PierTextField.qml`
  - `PierSearchField.qml`
  - `PierScrollView.qml`
  - `PierScrollBar.qml`

## Review rule

- Any UI PR should be rejected if it adds local control styling that belongs in `qml/components/`.
