# 数据格式与兼容性契约

> Draft 1  
> 原版规格基线：TooonyChen/finallyshell @ 78c2f91603bfb014b800a88cfb68fded64934141  
> 本文是 Rust 重写的兼容性验收规格。所有格式均以原版文档和实现为依据；遇到文档/代码不一致时，以测试和实际序列化行为复核后更新本文。

## 1. 配置目录定位

默认配置根目录遵循各平台用户配置目录，并追加 finallyshell：
- macOS：~/Library/Application Support/finallyshell
- Windows：%APPDATA%/finallyshell
- Linux：$XDG_CONFIG_HOME/finallyshell；未设置时 ~/.config/finallyshell
- FINALLYSHELL_DIR 存在时优先使用它。

路径处理必须覆盖：目录不存在、只读目录、权限不足、路径含 Unicode/空格、符号链接、并发启动和异常退出。

## 2. 持久化文件清单

| 文件/目录 | 数据 | 同步 | 关键行为 |
|---|---|---|---|
| hosts.json | 主机、文件夹、代理、隧道、监控网卡选择等 | 是 | v1 → v2 迁移；未知字段保留；原子写入 |
| hosts.v1.json | 迁移前 v1 原件 | 否 | 迁移成功前不得覆盖原件 |
| hosts.json.broken-* | 损坏配置备份 | 否 | 原始字节副本；不得静默覆盖 |
| settings.json | 全局选项 | 部分 | 仅保存非默认值；未知字段保留策略需验证 |
| known_hosts | SSH 主机密钥信任记录 | 否 | OpenSSH known_hosts 兼容；key 变化拒绝 |
| history/<host-id>.txt | 每台主机命令历史 | 否 | UTF-8，一行一条；重复命令移至末尾；超过 10 MiB 清理旧记录 |
| history/local.txt | 本地终端命令历史 | 否 | 与主机历史隔离 |
| quickcommands.json | 快捷命令分组和命令 | 是/需按原同步实现确认 | version 1；顺序有意义；未知键/更高版本保留 |
| commandeditor.txt | 命令编辑器原文 | 否 | UTF-8 原样保存，延迟写入 |
| sync-state.json | 上次同步的 remote/base/device/time/secrets 状态 | 否（本地状态） | 自身加密；删除后按首次同步处理 |
| hosts.before-sync.json | 同步修改 hosts 前的备份 | 否 | 同步前备份 |
| background-* | 终端背景图片副本 | 本地 | 文件名在设置中引用 |
| icon-themes/*.vsix | 用户安装的 VSIX | 本地 | 只解析数据，不执行扩展代码 |
| geo/* | ip2region 数据与在线查询缓存 | 本地 | 下载校验和、缓存 |
| updater.json | 更新器偏好 | 本地 | 原版由 MyGo 更新器管理，路径与普通配置目录有差异 |

## 3. hosts.json

### 3.1 顶层版本

v1：根节点是 Host 对象数组。  
v2：根节点是对象：
- version: 2
- folders: Folder 数组
- hosts: Host 数组

迁移要求：
1. 读取 v1 时生成内存中的 v2 模型。
2. 保留 v1 原文件为 hosts.v1.json。
3. 首次持久化时写入 v2；不能因仅启动/读取就破坏原件。
4. 空文件、非法 JSON、错误类型、重复 ID、未知版本均须有显式测试和可解释错误策略。

### 3.2 Folder

已知字段：
- id: 稳定 ID
- name: 显示名
- parent: 父文件夹 ID；缺省表示根层

文件夹可嵌套。删除、移动、重命名和 host.folder 引用一致性必须作为领域规则测试；循环父子关系必须拒绝或按原版行为处理。

### 3.3 Host

已知字段与语义：

| 字段 | 类型/值 | 缺省行为 |
|---|---|---|
| id | string | 稳定主机 ID |
| name | string | 显示名 |
| addr | string | SSH 主机地址 |
| port | integer | SSH 默认端口 |
| user | string | 登录用户名 |
| auth | password / key / agent | 登录方式 |
| keyPath | string | 私钥路径 |
| folder | string | 根目录为空 |
| notes | string | 备注 |
| noMonitor | bool | false：启用远端监控/exec |
| lastConnectedAt | timestamp | 未连接过 |
| deletedAt | timestamp | 无：未删除 |
| createdAt | timestamp | 旧记录可缺省 |
| termType | string | xterm-256color |
| backspace | del / ctrl-h | 跟随全局设置 |
| delete | vt220 / del / ctrl-h | 跟随全局设置 |
| encoding | utf-8 / gbk / gb18030 / big5 / shift_jis / euc-jp / euc-kr / iso-8859-1 / windows-1252 | 跟随全局设置；未知值按 UTF-8 |
| noRememberPassword | bool | false：记住密码 |
| monitorIfaces | string[] | 自动选择流量最大网卡 |
| proxy | Proxy object | 直连 |
| tunnels | Tunnel[] | 无隧道 |

Host 和嵌套对象的未知字段必须往返保留。实现时优先使用显式已知字段 + extension map/raw JSON，不要在反序列化时把对象收窄为固定 struct 后丢失未来字段。

### 3.4 Proxy

- SOCKS5：kind=socks5，addr、port，可选 user；密码进入凭据库。
- HTTP CONNECT：kind=http，addr、port，可选 user；密码进入凭据库。
- Jump：kind=jump，jumpHost 指向另一个 Host ID。
- proxy 缺省为直连。

必须验证：引用不存在、循环跳板链、多层代理/跳板混合、每一跳认证失败、每一跳 host key 首次确认、关闭根会话时整条链清理。

### 3.5 Tunnel

- local：本机监听，经 SSH 连接向目标转发。
- remote：远端监听，转回本机再连目标。
- dynamic：本机 SOCKS5 listener；不含 targetHost/targetPort。
- listenHost 缺省为 127.0.0.1；remote 的监听地址受限为 IP 或 localhost。
- disabled=true 表示连接时不自动启动。
- 端口范围 1..=65535；监听地址冲突必须明确报错。
- 未知字段保留。

## 4. settings.json

设置文件只写非默认值。未知键保留策略必须与原版实测确认。设置分两类：

### 4.1 同步设置

原版通过 syncedSettings/localSettings 两组列表决定同步范围。Rust 实现必须用明确白名单，不允许“整个 settings.json 一起同步”。

需要从原版 config/sync.go 和 tests 提取完整同步白名单；本地设置至少包括：
- 窗口布局尺寸和比例：sidebarWidth、monitorPercent、terminalPercent、treeWidth、procListPercent、procFieldsPercent、netListPercent、rightPanelWidth、rightMonitorPercent、commandEditorWidth、commandEditorPercent
- 本机 UI 状态：monitorHidden、bottomPanelHidden、bottomPanelSide、monitorSide、commandEditorShown、leftPanelPage、connectionTreeExpanded
- 本机路径/设备能力：termBackground、localShell、sshAgent、downloadDir、askDownloadDir、fileIconTheme
- 同步配置本身：sync
- 本地地理位置设置/缓存开关：geoLocations、geoLocationsV6、geoOnline

最终列表以源代码 syncedSettings/localSettings 为准，必须建立逐键测试，保证新增设置不会意外同步。

### 4.2 设置行为

需保留的关键设置类别：
- language、appearance
- close/tray/新会话选中策略
- keepAliveSeconds、sshAgent
- downloadDir、askDownloadDir
- terminal encoding/backspace/delete、字体/行高/粗细/连字、主题/背景
- mouse、快捷键、右键行为、滚轮
- 命令栏、路径补全、历史、命令补全
- server history、agent 标签图标、跟随 shell 目录
- file icon theme
- 面板布局、尺寸和可见性
- geo 查询偏好
- sync provider 和自动同步策略

未知枚举值必须按原版定义的安全默认值处理，不能 panic。

## 5. quickcommands.json

已知结构：
- version: 1
- groups: [{id, name}]
- commands: [{id, group?, name, text, enter?}]

语义：
- groups/commands 数组顺序即 UI 顺序。
- command.group 是 group ID；无效或缺省分组按“未分组”处理。
- name 为空时以 text 第一行作为按钮名。
- text 保留多行；末尾空行不发送。
- enter 缺省按 true。
- 未知键和更高 version 必须保留。
- 解析失败先备份 quickcommands.json.broken-*，不覆盖损坏原件。

## 6. 命令历史

- 每个 Host ID 一个 UTF-8 文本文件；本地终端固定 local.txt。
- 每行一条命令，旧到新；重复项重新移动到末尾。
- 空命令和控制字符命令不保存（Tab 除外）。
- 大于 10 MiB 时删除较旧记录。
- 删除主机到回收站不立即删历史；永久删除时清理。
- 不参与云同步。
- 文件可能被用户在程序关闭时手工编辑；运行期间首次需要时再加载，需测试外部编辑时序。

## 7. 凭据库

服务名固定 finallyshell。键约定：
- host/<host-id>/password
- host/<host-id>/passphrase
- host/<host-id>/proxy-password
- sync/password
- sync/<provider>/credential

平台后端：
- macOS Keychain generic password
- Windows Credential Manager generic credential
- Linux Secret Service（libsecret）

行为约束：
1. 凭据绝不进入 hosts.json/settings.json/普通日志。
2. 记住密码关闭时从系统库删除，只在本次运行内存保留。
3. 认证失败的旧凭据不能直接覆盖；仅在成功登录后替换。
4. 凭据库不可用时退化为内存，并让 UI 知道不可持久化。
5. 凭据 API 可能阻塞或触发系统授权，必须在后台执行；要有串行化/并发访问策略。
6. 主机复制、删除、恢复、代理更改时，凭据键迁移/复制/删除必须有行为测试。

## 8. 云同步 Envelope 与 Payload

### 8.1 Envelope format 1

JSON 字段：
- app = "finallyshell"
- format = 1
- kdf = "argon2id"
- salt = base64 bytes
- t = 3
- m = 65536 KiB
- p = 4
- cipher = "aes-256-gcm"
- nonce = base64 bytes
- data = base64 ciphertext + GCM tag

密码派生 32-byte key。AAD 为 UTF-8 字节串 finallyshell-sync/1。nonce 每次加密必须随机且不可复用。读取时按 Envelope 自带参数派生，不能把当前默认参数硬编码成唯一可读格式。未知更高 format 应拒绝并报告“由更新版本写入”，不能尝试降级解密。

### 8.2 Payload schema 1

字段：
- schema = 1
- device、updatedAt
- folders、hosts：对象数组，按 id 合并，未知字段原样保留
- settings：按 key 合并的 JSON map
- secrets：可选，按凭据键合并

### 8.3 三方合并规则

对每个 folder/host ID、setting key、secret key，以 base/local/remote 比较：
- 仅一侧变化：采用变化的一侧。
- 一侧删除、另一侧未变：删除。
- 两侧都变化且不同：采用本次同步设备的 local 值，并计一个冲突。
- 无 base 的首次同步不推断删除；双方都有时保留 local。
- folder/host 输出顺序：local 原顺序，然后追加 remote 独有项。
- 无 ID 的对象只保留 local，不从 remote 合并。
- JSON 对象键顺序/空白差异不构成内容变化。

### 8.4 Provider 和并发写入

- Gist、WebDAV、S3 兼容服务。
- S3 使用 SigV4；endpoint/region/bucket/key/access key ID 属于配置，secret access key 属于 credential store。
- 必须测试 ETag/条件更新、远端在读写之间变化、冲突重试、超时和部分失败。
- sync-state.json 的 base 是合并正确性基础；不能只做无状态上传/下载。

## 9. Known hosts

- 文件采用 OpenSSH known_hosts 语法兼容。
- 首次见到 key：向用户请求明确确认，确认后追加规范化 host 条目。
- 已存在条目匹配失败且旧 key 存在：拒绝连接并报告指纹变化。
- 并发首次连接不能互相覆盖写入；追加操作要串行化。
- 不允许“自动接受变化的 host key”。

## 10. Golden fixture 与测试计划

在 Rust 仓库建立纯测试数据，不从原仓库复制实现代码。建议布局：

- tests/fixtures/config/hosts-v1.json
- tests/fixtures/config/hosts-v2-full.json
- tests/fixtures/config/hosts-unknown-fields.json
- tests/fixtures/config/hosts-malformed.json
- tests/fixtures/config/settings-mixed.json
- tests/fixtures/config/quickcommands-v1.json
- tests/fixtures/sync/envelope-format-1.json
- tests/fixtures/sync/payload-schema-1.json
- tests/fixtures/sync/merge-cases/
- tests/fixtures/known-hosts/

必须覆盖：
1. v1 → v2 迁移、迁移备份、重复/缺失 ID、未知字段往返。
2. 原子写入故障注入：临时文件创建失败、写入失败、rename 失败、进程中断后恢复。
3. 凭据库正常、缺失、拒绝、取消、锁定、并发请求。
4. Envelope 跨语言互通：Go 生成 Rust 解密、Rust 生成 Go 解密；错误密码、篡改 nonce/data/AAD、无效参数和未来版本。
5. Merge 表驱动覆盖新增/修改/删除/双边冲突/首次同步/顺序/无 ID/JSON canonicalization。
6. provider 的条件写入与冲突重试。
7. 同步白名单和本地设置黑名单的双向断言。

## 11. 尚待从源码/测试确认

- settings.json 对未知字段的精确保留行为。
- syncedSettings 的完整白名单与设置变更期间的同步合并时序。
- hosts 删除/恢复/永久删除时凭据与历史的完整清理规则。
- Envelope 的随机数错误处理、参数上限、key cache 生命周期。
- 各 provider 对 ETag/版本冲突和重试的具体行为。
- known_hosts 对 hashed host、非默认端口、IPv6 和并发写入的细节。
- quickcommands 更高版本及未知字段在所有失败路径上的处理。

这些条目要通过 Go 源码和测试复核后标记为 Frozen；当前不能仅凭文档推定。
