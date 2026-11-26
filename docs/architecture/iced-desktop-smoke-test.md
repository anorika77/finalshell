# Iced 原型：真实桌面 Smoke Test

> 状态：待真实桌面执行  
> 原型：`spikes/ui-iced-smoke`  
> 基线 commit：`81d06dfc`  
> 适用平台：Windows 11、macOS、Linux X11、Linux Wayland  
>
> 本清单用于记录真实窗口系统中的交互结果。GitHub Actions 的 headless build/test 不能替代本清单。

## 1. 结果记录规则

每个平台单独记录，不跨平台推断。

- `PASS`：按步骤执行成功，并记录系统版本、桌面环境和复现条件。
- `FAIL`：存在可复现问题，附步骤、预期/实际结果和日志。
- `N/V`：尚未执行。不得将未执行标记为通过。
- `N/A`：该平台/环境不适用，并说明原因。

建议记录环境：

| 字段 | 值 |
|---|---|
| OS / 版本 | N/V |
| 架构 | N/V |
| 桌面环境 / Window Server | N/V |
| 显示缩放 / DPI | N/V |
| 输入法 / 版本 | N/V |
| Iced spike commit | N/V |
| 执行日期 | N/V |

## 2. 输入法与文本输入

### IME composition

- [ ] 在 Connections Filter 中输入中文拼音，检查 preedit 组合文本是否正确显示。
- [ ] 移动候选选择并确认上屏；确认没有丢字、重复字或意外提交。
- [ ] 在 Quick command 输入框重复上述测试。
- [ ] 测试日文假名转换与韩文组合输入。
- [ ] 窗口缩放、移动窗口、切换焦点后再次呼出候选窗，检查候选窗位置是否跟随光标。
- [ ] 连续中英文切换，检查光标位置、selection 和组合状态。

### Focus 与快捷键隔离

- [ ] Filter 输入框获得焦点时，输入字母 h/v/w，不应触发分屏或关闭 pane。
- [ ] Quick command 输入框获得焦点时，输入普通字符和组合字符，不应被 workspace shortcut 截获。
- [ ] 输入框聚焦时，验证 Ctrl/Command+Shift+H/V/W 的处理符合产品预期；若控件优先消费事件，应记录实际行为。
- [ ] 点击 terminal pane 后再触发分屏/关闭快捷键，确认只作用于预期 pane。
- [ ] 快捷键长按时不应因 key repeat 连续创建多个 pane 或连续关闭 pane。
- [ ] macOS 验证 Command 映射；Windows/Linux 验证 Ctrl 映射。

当前快捷键映射：平台 command modifier + Shift + H/V/W；平台 command modifier + Alt + 1/2/3。该映射仍是原型行为，不代表最终产品快捷键方案。

## 3. Pane 与 Session 生命周期

- [ ] 创建第二个 pane，确认新 pane 获得新 Pane ID 和 Session ID。
- [ ] 对同一个 pane 连续 split，确认既有 pane 的 Session ID 不变。
- [ ] 拖动 pane 到另一个位置，确认 pane 内容与 Session ID 一起移动，不发生串线。
- [ ] 拖动 divider 调整大小，确认相邻 pane 尺寸变化合理。
- [ ] 关闭一个 pane，确认其他 pane 仍存在且身份不变。
- [ ] 关闭当前焦点 pane，确认焦点落到仍存在的 pane。
- [ ] 仅剩一个 pane 时尝试关闭，确认不会意外关闭整个 workspace。
- [ ] 快速连续 split / drag / close，检查是否出现失焦、重复 pane 或错误身份。

注意：当前原型使用 deterministic fixture，并未连接 SSH/PTY。这里验证的是 UI pane 状态与模拟 Session 身份，不是远端连接生命周期。

## 4. 列表与筛选

### Connections（1,000 条 fixture）

- [ ] 连续滚动到列表末尾，确认没有空白区或跳行。
- [ ] 输入精确主机名筛选，再清空筛选，确认滚动位置合理。
- [ ] 在列表中部选择一项，修改筛选条件后确认选中状态按 Host ID 保留。
- [ ] 输入无匹配条件，确认空列表布局正常。
- [ ] 快速改变筛选条件并滚动，确认视口位置和渲染行数稳定。

### Remote files（10,000 条 fixture）

- [ ] 从顶部滚动至中部和末尾，确认行内容、行高和滚动位置一致。
- [ ] 在中部选择文件，滚动离开再返回，确认选择状态保留。
- [ ] 连续拖动滚动条到末尾，确认最后一行可见且没有不可达区域。
- [ ] 缩放窗口高度后滚动，确认可视窗口与实际内容位置一致。

## 5. 监控面板与窗口行为

- [ ] Overview / Network / Processes 切换正确。
- [ ] 使用快捷键切换监控 tab 时，焦点和当前 pane 不发生意外变化。
- [ ] 调整主窗口大小，三栏布局无重叠、异常裁切或不可操作控件。
- [ ] 测试窗口最小尺寸和最大化/还原。
- [ ] 测试 100%、150%、200% 显示缩放（平台支持时）。
- [ ] 测试浅色/深色系统主题；当前原型主题能力不足的项目标记 N/V，不推断通过。
- [ ] 测试多显示器间移动窗口，检查 DPI 切换后的布局和文字清晰度。

## 6. 明确不在当前原型验收范围内

以下能力尚未实现，不能在本轮标为通过：

- 真实 SSH、PTY/ConPTY、SFTP 与远端文件操作。
- 真实终端 VT 渲染、滚动历史、鼠标协议、剪贴板和终端 selection。
- 连接树节点折叠、树内拖放排序和持久化。
- 文件列表排序、筛选、远端目录导航。
- Secondary windows、系统托盘、原生菜单、安装包和自动更新。
- 无障碍语义树与 Screen Reader 完整验证。
- 真实网络负载下的 UI 背压与终端高频输出性能。

## 7. 当前自动化证据边界

GitHub Actions Iced workflow 覆盖五个目标 runner：

- macOS ARM64
- macOS x86_64
- Linux x86_64
- Linux ARM64
- Windows x86_64

自动化目前验证格式、测试和 release build。它不等于在上述系统的真实桌面会话中完成 IME、候选窗定位、DPI、拖放、托盘或辅助功能测试。

完成真实桌面测试后，应将每个平台的结果、复现步骤和必要截图/录屏链接追加到本文件，再进入 GPUI CE 同规格原型对照。
