# Rust + nsh 迁移计划

目标是重写 PeaZip，交付 macOS、Linux、Windows、Android、iOS、Web；下面的阶段不是缩减最终范围。保留 Pascal 源码用于行为对照和上游跟进。未通过的平台不得标记已支持。

## 阶段与验收

| 阶段 | 工作 | 完成依据 | 状态 |
| --- | --- | --- | --- |
| 1 | Rust workspace、内存 ZIP 核心、安全边界、上游基线与 CI | 正常归档往返、恶意路径及限额测试；基线报告可执行 | 本地验证完成；远程 CI 待运行 |
| 2 | nsh Rust 界面：打开、列表、选择、创建、解压、错误反馈 | 用真实归档操作界面；UI 状态测试与本机启动验证 | 已有桌面/Web 与 Android ZIP 基本流程；更多格式、iOS 功能及剩余外部 API 验收待完成 |
| 3 | 安全文件系统与平台文件选择、后台任务、进度、取消、覆盖冲突 | 符号链接/竞态/失败清理测试；大文件不中断界面 | 已有有界读取、后台操作、新目标原子不覆盖发布、Web 创建取消与阶段进度；桌面取消/进度、其他冲突策略/流式大文件待完成 |
| 4 | 上游格式与功能逐项迁移 | 格式/操作能力清单；跨工具兼容样本、加密/分卷/损坏文件测试 | 待完成 |
| 5 | 文件管理、搜索、预览、校验、转换、分割、配置、语言与无障碍 | 按上游功能回归；权限和交互验证 | 待完成 |
| 6 | 六平台构建、打包与发布 | 每个平台真实构建、运行、文件导入导出和归档往返；发布物可安装 | Web 浏览器流程、Android arm64 调试 APK 安装/导入已验证；iOS SDK 构建、移动导出及各平台发布验收待完成 |
| 7 | 持续跟进上游 | 定期差异报告、映射到回归测试、审查并更新基线 | 持续 |

## 工程边界

- `crates/peazip-core`：不依赖界面和平台文件系统，接受字节流；原生与 Web 共用归档语义。
- `crates/peazip-native`：原生文件系统适配；当前提供有界读取选中文件、临时文件写入与禁止覆盖发布。UI 只在桌面目标链接此 crate。
- nsh 源码 `/Volumes/MacData/Users/xxnuo/Projects/nsh` 只读。接入时固定 revision 并记录平台限制，禁止为了编译修改依赖源码。
- 文件系统提取必须单独处理链接、覆盖、原子发布和失败清理；内存路径校验不等价于安全落盘。
- 不支持的格式/操作必须明确反馈；不能把首个 ZIP 功能称为完整 PeaZip。
- 每个完整功能单独 commit。测试与实际结果一起更新状态，不能只根据编译通过判定产品完成。

## 上游跟进

`upstream-baseline.json` 固定已审查的 PeaZip 与 nsh revision。运行 `uv run python scripts/upstream_report.py` 输出相对本地上游引用的提交和文件差异；先用 `git fetch upstream` 更新引用。报告不会改源码或自动推进基线。

处理每次上游更新：审查差异 → 定位对应 Rust 功能（尚未迁移的加入阶段清单）→ 补回归样本/测试 → 实现并验证 → 更新基线并提交。基线仅表示已审查的来源，不能表示功能已全部迁移。共享分支更新使用 fetch + rebase，不覆盖他人提交。

CI 先验证三个桌面系统的核心测试与 Web 编译；这不等于 UI、移动平台或发布物已经验证。后续必须增加各平台实际运行与打包作业。

## nsh 已确认的接入约束

源码基线提供纯 Rust 的桌面入口和 CLI 生成的 Web/Android/iOS runner。复用 `examples/tasks-rs` 的组件和桌面宿主，关闭平台 crate 默认 features，避免引入不需要的 JS 引擎。

- Web 的 `blocking`/`background` 不可用，压缩需要 Worker 或可让出执行的方案；OPFS 不等价于原生路径 IO。
- Android 当前文件服务没有保存/目录对话框；需要 SAF 文档和目录授权适配。
- iOS picker 导入副本；导出与持久外部目录授权需要单独适配。
- 通用 picker 会整体读取文件，512 MiB 限额不能充当大归档方案。
- Linux 需 GTK 4.14+ 等系统开发库；移动端分别依赖 Android SDK/NDK/JDK 和 Xcode/签名。缺少工具链的构建列为未验证。

这些是待完成工程工作，不能因此删减平台目标。

## 验证记录

- 2026-11-07：核心 10 个集成测试通过，涵盖 Unicode/目录/空文件往返、重复路径与冲突、链接、路径穿越、各项资源限额、压缩炸弹、CRC、声明尺寸不实、建索引前的条目限额。核心 Clippy 与 wasm32 编译通过。
- 上游报告测试在临时 Git 仓库覆盖新增提交、无变化、无效引用、不相关历史；当前 PeaZip 上游与初始基线一致。
- 默认内存模式限归档 32 MiB、4096 条目、单文件 16 MiB、总解压 64 MiB、压缩比 200。后续流式大文件路径必须独立实现，不能宣称这些默认值已满足完整 PeaZip。
- ZIP 库固定 `=9.4.1`，因为有界目录迭代器目前位于 unstable API；升级必须重新验证安全测试。
- 2026-11-07：`cargo test --workspace --locked` 共 13 项通过；全 workspace Clippy、格式检查通过。UI library 与核心的 wasm32 编译通过。UI headless 流程覆盖打开真实 ZIP、取消、损坏/缺失文件恢复和大小限额。
- macOS `cargo run -p peazip-ui --locked` 启动成功，系统窗口列表确认进程创建了 840×632 的可见窗口；窗口截图失败，视觉与人工交互尚未验证。测试启动的进程已关闭。
- Web/Android/iOS 界面已接通通用文件 picker，桌面使用路径选择与后台读取。Web 当前在主线程解析有界目录；移动/Web picker 会先读文件，尚不能在读入前应用核心的 32 MiB 限额。浏览器与移动真机运行仍待验证。
- 远程 CI 已配置桌面核心/UI测试、Web编译和每周上游差异报告，但本轮未推送、未取得远程运行结果。

- 归档选择：按路径维护选择，虚拟列表使用可访问的 checkbox；全选、清空、取消/打开失败保留选择、成功打开归档重置选择的 headless 测试通过。UI 共 4 项测试；Clippy 与 wasm32 library 编译通过。
- 桌面创建：选择多个常规文件，以 basename 写入新 ZIP；后台处理并在保存后显示内容。取消任何对话框不写文件；成功创建重置选择，失败保留当前归档。现有目标（含链接/目录）不会覆盖。两个新 headless 测试验证真实内容往返、源文件不变、拒绝覆盖、取消和重试。
- 原生适配：拒绝末级链接和特殊文件，按单项/总量限额读取；同目录临时文件同步后使用 `persist_noclobber` 发布。源/目标父目录是用户选择的可信位置，尚未宣称防御父目录被恶意替换。Windows reparse 分支尚待 Windows 运行验证。
- 创建高重复数据时若 Deflate 超过读取压缩比限额，最多重试一次 Stored，读取恶意高压缩比归档仍拒绝；测试验证 1 MiB 全零数据和输出大小边界。
- 本轮集成验证：12 项 core、6 项 native、6 项 UI 测试全部通过；workspace Clippy、格式检查和 core/UI wasm32 编译通过。CI 已加入 native 在三个桌面系统上的测试。远程 CI、移动端和浏览器运行证据仍缺失。
- Windows 路径发布先规范化父目录，避免 tempfile 直接调用 Win32 时触发传统路径长度限制。`cargo check -p peazip-native --tests --locked --target x86_64-pc-windows-gnu` 通过，含 Windows 长路径测试的编译；这不代表该测试已经在 Windows 上运行。
- 选中解压：目录选择包含子项，重叠选择不会重复输出，未知条目报错；全包元数据仍先通过安全检查，仅解码所选载荷并校验 CRC。当前仍按全包声明大小限制资源，未声称支持超大归档的局部读取。
- 桌面解压：保存对话框选择新的输出目录名称，先在内存验证所选载荷，再写同父目录随机暂存目录；Unix 在创建时应用 0700。Linux/macOS/Windows 分别使用原子不覆盖重命名操作，已有文件、目录（含空目录）与链接都不会覆盖；发布成功后取消旧临时路径的析构清理。普通错误路径清理暂存，进程被终止或清理操作自身失败的残留恢复尚待实现。
- 界面持有打开/创建后的不可变 ZIP 快照，提取不会重开原路径；源文件被替换或删除后仍提取所见内容。当前操作期间禁用其他归档操作和选择；失败/取消保留选择以便重试。原生打开已复用安全常规文件读取，拒绝末级链接/FIFO等特殊文件，防止阻塞后台线程。
- 本轮验证：core 15 项、native 15 项、UI 8 项测试通过；包括 CRC 错误无输出、发布竞争、写入中途失败清理、部分/子目录解压、取消与快照一致性。全 workspace Clippy、Web library 编译通过；native 及测试在 Windows GNU、Linux GNU 目标交叉编译通过。跨平台运行、真实保存对话框及完整视觉验收仍未完成。
- 当前兼容边界：部分文件系统不接受的长单组件名、Unicode别名或 Windows 特殊保留名可能通过归档列表但提取失败；`create_new` 和原子发布防止覆盖，之后需补齐平台文件名冲突处理。Windows 暂存权限继承可信父目录 ACL，与 Unix 0700 不等同。

- 目录创建：native 接口支持文件/目录混选，保留根目录名与空目录，稳定排序归档条目；界面增加单独的文件夹选择入口，因为 nsh 的混合文件/目录选择仅部分平台支持。工具栏允许换行，窄窗口仍可操作解压按钮。
- 递归读取使用固定版本 `cap-std`/`cap-fs-ext` 4.0.3，子项相对已打开目录句柄并禁止跟随链接；目录被替换为链接的受控测试确认不会读到外部目标。按需打开子项、逐个处理选中根节点，避免同时持有全部文件或父目录句柄；在句柄上限 64 的子进程内归档 512 个选中文件通过。
- 递归资源限额在遍历/入队及读取时执行，空目录也计入条目上限；链接循环、特殊文件、重复根名、过长路径和超量内容均失败且不发布。输出放在源目录内时，与放在目录外的 ZIP 逐字节一致，不包含自身或暂存文件。
- 本轮验证：core 15、native 21、UI 10 个测试函数通过（低句柄限制子进程额外执行 1 次）；workspace Clippy/格式检查、UI wasm32 编译、native Windows/Linux 测试交叉编译均通过。UI 流程验证目录选择对话框类型、中文/空目录创建与解压、取消。尚无 Windows/Linux 运行时及浏览器实际运行证据。

- Web 宿主：`crates/peazip-web` 使用 nsh 的纯 Rust `rust_app!` 入口与 8 MiB 栈，复用现有 UI/core；未引入 TSX 或另一套界面。构建时从 Cargo 锁定的 nsh checkout 复制对应 JavaScript 宿主，不依赖本机只读源码路径。
- `uv run python scripts/build_web.py` 输出优化静态产物到 `target/web-dist`（约 12.5 MiB WASM，另含宿主 JS、HTML 和项目/nsh 许可证）；`--debug` 供开发，调试模块约 535 MiB，不作为交付产物。此目录与 Cargo 的 profile 缓存分离。可用 `uv run python -m http.server 8765 --bind 127.0.0.1 --directory target/web-dist` 本地运行。
- 浏览器验证：`pnpm --dir web install --frozen-lockfile`、`pnpm --dir web exec playwright install chromium`、`pnpm --dir web test`。真实 Chromium 加载优化 WASM，覆盖 Python ZIP 样本的中文名称/大小/空目录、单选/全选/清空、错误归档恢复、超出 32 MiB 的输入拒绝与空输入。没有 console/pageerror 异常；空输入不等同于已验证操作系统文件对话框的真实取消。已检查初始暗色界面及导入后的亮色截图。
- 新宿主通过 wasm32 Clippy，现有 workspace 测试、Clippy、格式检查和上游报告测试通过。CI 新增优化 Web 构建、真实 Chromium 测试及静态产物上传；尚未推送/运行远程 CI。本次仅本地验证，没有发布网站。

- Web 创建：浏览器选择多个文件，先检查数量、名称长度、16 MiB 单文件及 64 MiB 总量，再交给独立 Worker 读取与压缩，避免创建操作先把文件载荷读入 UI WASM。`peazip-worker` 使用与桌面相同的 Rust core，通过有界二进制协议再次检查输入；创建完成更新归档快照，用户点击 Download ZIP 发起下载，不宣称浏览器已经写入磁盘。
- Worker 在成功、错误或页面离开后释放；取消/错误保留原快照。回调绑定任务身份，避免旧请求结果完成新任务。缓存页面离开会结束 Rust pending；真实 bfcache 往返尚未验证，目前测试为连续两次合成 persisted pagehide 后可重新创建。
- 新构建工具要求：先运行 `cargo install wasm-bindgen-cli --version 0.2.130 --locked --root target/web-tools`，再运行 `uv run python scripts/build_web.py`。脚本检查 CLI 与 Cargo 锁定版本一致，使用官方胶水处理 Worker 依赖的 wasm-bindgen imports，不使用抛错占位导入；不会覆盖用户已有全局 CLI。优化 Worker 模块约 275 KiB，主界面约 12.5 MiB。
- 本轮验证：3 项 Worker 协议测试（截断/边界/UTF-8/重复/尾随字节/真实 ZIP 往返），现有 Rust workspace 回归及 native/wasm Clippy、格式检查通过。2 项真实 Chromium 流程通过，下载经 Python zipfile 独立验证后重开；取消及数量/大小超限不启动 Worker，重名失败恢复，忙时禁用按钮，7 个实际 Worker 全部释放。CI 增加 Worker 测试与匹配 CLI 安装。

- Web 创建任务新增 Cancel：仅创建进行时可操作，终止 Worker 并结束 Rust pending，不触发下载；保留原归档字节和选择。读取进度来自实际完成的文件数量（最多 100 次通知），随后显示压缩阶段，不宣称读取比例就是压缩比例。
- 进度通过 Rust 异步通道进入 nsh executor 后再更新 signal；不能在 JavaScript 调用的 WASM 回调内直接修改 signal，因为此时未进入 nsh reactive runtime。旧任务消息仍受身份检查保护。
- 本轮 4 项真实 Chromium 流程通过：WASM 加载阶段及 24 MiB 真实压缩阶段主动取消，Worker 终止且无成功输出，原 ZIP 字节/选择保持不变，同批文件重试成功；251 文件仅发出 84 条递增读取进度。现有 Rust workspace 测试、native/wasm Clippy、格式检查通过。

## Web 目录导出的边界与验证

浏览器 [getFileHandle/getDirectoryHandle 的 create](https://fs.spec.whatwg.org/#api-filesystemdirectoryhandle-getfilehandle) 不提供原子“不覆盖”，已有条目会返回其 handle；[目录 picker](https://wicg.github.io/file-system-access/#api-showdirectorypicker) 还要求用户激活和授权。不能复用桌面原子发布的保证描述 Web 实现。

现有实现使用用户授权父目录下的随机新子目录，保留所选目录树与空目录；先在 Worker 完成 core 校验与解压，任何已发现冲突立即停止。只有全部文件提交才显示完成；失败准确报告输出名、完成数、可能未完成路径及残留，不自动递归删除可能含外部新增文件的目录。并发外部修改时 API 无法提供严格 no-replace；单文件真实字节下载仍可作后续兼容补充，不能冒充完整目录导出。测试分别记录文件 API 行为、外部目录授权及失败状态，不能用 OPFS 测试替代全部平台验收。

- Worker 新增选中解压协议，直接返回经 CRC 校验的真实文件字节与目录条目，不重新打包 ZIP；支持目录子项选择、未知/恶意/截断/损坏输入拒绝。JS 写入前再次验证整个输出包及目录树预算。
- core 解压预算现在包含隐式父目录，防止少量条目路径生成超量目录；共享父目录去重，大小写不同的实际路径分别计数。该检查也用于桌面提取。
- Web 在用户操作期间请求目录授权，不支持目录 picker 时禁用提取并显示原因。提取使用已打开的归档快照，解压期间可终止 Worker；写入期间取消会请求 abort 并等待实际结果，close 已开始时不承诺撤销提交。取消/失败保留原归档与选择。
- 本轮验证：core 16、worker 6、native 21、UI 10 个测试函数及低句柄子进程通过；native/wasm Clippy、格式检查通过。JavaScript 32 项通过：9 项 Chromium 流程（含真实 Rust Worker/OPFS）和 23 项注入文件系统故障的单元测试。
- OPFS 实测包括真实解压字节、空目录、未选内容不导出、sentinel 保持不变、CRC 失败无输出、权限/取消恢复、能力缺失提示及加载期取消；写入期取消测试在真实 OPFS writer 外注入等待，验证 abort 和残留空文件报告。目录 picker 被测试替换为 OPFS 授权入口，**没有验证原生外部目录选择器、真实磁盘权限或外部修改竞态**，这些仍是待验收项。

## 移动宿主与构建验证

- Android 与 iOS 宿主复用同一 `peazip-ui`，固定 nsh revision 并关闭 JS 默认 features；移动 app ID 为 `io.peazip.rust`。根布局增加安全区域留白，避免按钮与状态栏/手势区重叠。
- `uv run python scripts/build_android.py` 使用锁定依赖中的 Java 源码，执行 Rust、javac、d8、aapt2、zipalign 和 apksigner；无需 Gradle，也不修改本机 nsh 源码。支持 `--sdk`、`--ndk`、`--jdk`、`--abi`；默认优化 Rust 构建，产物 `target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 为 **debug 签名/可调试预览**，不是商店发布包。
- 本机 Android 实测使用 SDK 35、build-tools 36.0.0、NDK 29.0.13113456、Android Studio 自带 JDK，进程内配置工具路径。APK 签名验证及 16 KiB zipalign 检查通过，ELF LOAD 段对齐为 0x4000。仅 arm64-v8a 已实际构建，其他 ABI 选项尚未验证。
- `uv run python scripts/smoke_android.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 在独立 Android 36 模拟器上通过：实际安装/启动、系统 DocumentsUI 导入、中文名称和空目录、全选/清空及全部 checkbox 状态、取消 picker 保留内容、损坏归档报错后恢复。PID 保持稳定、无应用 AndroidRuntime fatal；最终 PNG/XML 在 `target/android-smoke`。脚本只删除自身随机测试文件；专用模拟器已关闭。当前只验证英文 API36 系统 picker，未验证真机和其他系统版本。
- `uv run python scripts/build_ios.py --target aarch64-apple-ios-sim` 已提供 SDK 查找、scoped Xcode、真实 Mach-O 架构/平台检查和 `.app` 打包；模拟器 ad-hoc 签名，真机输出明确未签名。**本机只有 Command Line Tools，Xcode 别名目标不存在，模拟器/真机 SDK 缺失，未生成或运行 iOS app**。5 项脚本测试验证环境边界和拒绝假平台产物，host stub 通过不能替代 iOS 编译。
- CI 新增 Android APK 与 iOS simulator app 构建/产物作业，尚未取得远程运行结果。Rust workspace 回归、Clippy、格式检查、6 项 Python 测试和 Web 32 项测试通过；只读 nsh 工作树保持干净。初始移动宿主仅提供导入、列表和选择；Android 后续功能见下，iOS 仍待 SDK 环境验证。

- Android 创建/保存：应用自有 `PeaZipActivity` 和 Rust JNI 适配复用 nsh 生命周期，不修改 nsh 源码。系统多选文件后先检查元数据数量/大小，再按实际字节有界读取；通过后台 Rust core 创建 ZIP，独立 Save ZIP 操作使用 `ACTION_CREATE_DOCUMENT`，关闭输出流成功才报告保存。
- 新输入桥只接受 content URI，未知或不可信大小仍受 16 MiB 单文件/64 MiB 总量限制；保存上限 32 MiB。AssetFileDescriptor 的 offset/length 是选中资产的边界，读取使用其输入流工厂；固定长度或非零 offset 的目标资产拒绝保存，防止越界或静默截断。保存失败报告可能残留，不自动删除用户文档。
- 大数组 JNI 转换与完成回调在后台执行，先清理 Activity 状态、释放传输名额，再完成 Rust future；generation/request ID 隔离旧回调。销毁时使用 CancellationSignal 并异步关闭跟踪资源，全局有界传输/取消/关闭执行器阻止资源累计。若 provider 的取消和关闭均不返回，唯一名额会持续占用、拒绝新创建/保存请求，需进程重启恢复；不声称能强制终止不合作的 provider。
- `uv run python scripts/check_android_bridge.py` 已加入 Android CI：直接引用生产 Rust/Java，运行 11 项有界读、14 项真实 JVM/JNI、7 项资产边界检查，以及 64 MiB 后台转换、旧任务清理期间拒绝 200 次新请求等生命周期检查。部分 Android 类为测试 doubles，不替代设备验证；这些检查独立于桌面 cargo test，避免强制桌面开发安装 JDK。
- 最终 APK 在 Android 36 arm64 模拟器通过扩展 smoke：多选中文名/二进制/空文件创建，17 MiB 输入拒绝且保留旧状态，取消保存不写目标，同名目标由当前 Documents provider 自动生成 `(1)`、sentinel 未覆盖；拉回 ZIP 用 Python zipfile 独立核对名称、内容和 CRC，重新导入成功。PID 稳定、无应用 fatal；新标题宽度与列间距已在长文件名截图检查。
- 本轮 Rust workspace 回归、主机及 Android target Clippy、格式检查、Web 32 项测试通过。专用模拟器已关闭，未部署或上传发布包。只实测本机 Documents provider，未宣称所有云 provider、保存失败和真实恶意 provider 场景都已设备验收。

- Android 目录解压：先在 Rust 后台完成所选 payload/CRC 校验，再通过 `ACTION_OPEN_DOCUMENT_TREE` 授权父目录；以 DocumentsContract 的 opaque ID 创建随机新子目录，保留所选子树、隐式父目录和空目录。Java 输出包再次校验路径、类型、重名、Unicode alias 和资源预算；创建返回的 authority/tree scope、名称、类型和 ID 均检查，普通文件 MIME 允许 provider 规范化。
- 目录查询的 Loading/非空 EXTRA_ERROR 不得作为“不存在”的依据；单字段与累计 provider 元数据也有额度，URI 按需构造，避免长 tree ID 在每行重复放大。任何已发现冲突、重命名或 ID alias 都停止；不会合并已有目录或自动删除残留。SAF 没有跨 provider/外部并发修改的原子 no-replace 保证，这不等同于桌面原子发布。
- 读写 helper 在关闭前记录 reliable descriptor 能力，关闭后检查缓存的远端错误；已可检测的 provider 失败不会再计入成功。仅能检测本端关闭时已报告的错误，不等同于云端事务提交确认。该修复同时覆盖 ZIP 保存与目录写入。
- `uv run python scripts/check_android_device.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 构建独立 instrumentation APK，运行后卸载测试包，生产入口没有测试后门。本机通过 7 项真实 Android reliable-pipe/普通文件检查，包括“普通 close 成功但远端已报错”场景。
- 本轮 hostchecks：86 项导出策略、25 项 JNI、13 项资产、11 项读取及生命周期检查通过；这些使用部分 Android doubles。最终 APK 全量真实 Android 36 DocumentsUI/SAF smoke 退出码 0：树 picker 取消无输出，CRC 错误在 picker 前拒绝；嵌套二进制和 txt 内容、空目录、未选文件缺席、sentinel 不变及错误后恢复均已拉回核对。最终本机运行 token 为 `cf3a77e3610d`，截图在 `target/android-smoke/<token>/extracted.png`。
- Rust workspace 回归、主机/Android target Clippy、格式检查和 Web 32 项测试通过；只读 nsh 保持未改动。Android 此处验证的是 API36 本机 Documents provider，不代表所有云 provider、真实外部竞态或进程被强杀后的恢复都完成。

下一步：扩展 TAR/GZIP/7z 等归档后端并推进大文件流式处理，同时继续解决 iOS SDK 构建/文件导出和 Windows/Linux 实际运行验收；补齐 Web 外部目录、桌面/移动端主动取消与进度、冲突策略及文件管理功能。Web/Android 打开归档仍走 nsh picker，创建使用独立有界入口；两者限制不可混为一谈。六平台实际运行、完整功能与发布验收仍是最终要求。
