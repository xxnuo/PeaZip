# Rust + nsh 迁移计划

目标是重写 PeaZip，交付 macOS、Linux、Windows、Android、iOS、Web；下面的阶段不是缩减最终范围。保留 Pascal 源码用于行为对照和上游跟进。未通过的平台不得标记已支持。

## 阶段与验收

| 阶段 | 工作 | 完成依据 | 状态 |
| --- | --- | --- | --- |
| 1 | Rust workspace、内存 ZIP 核心、安全边界、上游基线与 CI | 正常归档往返、恶意路径及限额测试；基线报告可执行 | 本地验证完成；远程 CI 待运行 |
| 2 | nsh Rust 界面：打开、列表、选择、创建、解压、错误反馈 | 用真实归档操作界面；UI 状态测试与本机启动验证 | 已有桌面/Web 与 Android ZIP 基本流程；更多格式、iOS 功能及剩余外部 API 验收待完成 |
| 3 | 安全文件系统与平台文件选择、后台任务、进度、取消、覆盖冲突 | 符号链接/竞态/失败清理测试；大文件不中断界面 | 已有有界读取、后台操作、新目标原子不覆盖发布、Web 创建取消与阶段进度；桌面取消/进度、其他冲突策略/流式大文件待完成 |
| 4 | 上游格式与功能逐项迁移 | 格式/操作能力清单；跨工具兼容样本、加密/分卷/损坏文件测试 | 已有 ZIP、TAR、TAR.GZ、7z 读取/选中解压；其余格式、加密/分卷等待完成 |
| 5 | 文件管理、搜索、预览、校验、转换、分割、配置、语言与无障碍 | 按上游功能回归；权限和交互验证 | 待完成 |
| 6 | 六平台构建、打包与发布 | 每个平台真实构建、运行、文件导入导出和归档往返；发布物可安装 | Web 浏览器流程、Android arm64 调试 APK 的导入/创建/保存/目录解压已验证；Linux GTK/X11 实际导入/解压已验证；iOS 构建与导出、Windows 实际运行及各平台发布验收待完成 |
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

## 格式后端推进

- 新增内容识别和通用 `list_archive` / `extract_archive` / `extract_selected_archive`，保留 ZIP 专用 API。未压缩 TAR 复用路径、重名/父子冲突、实际字节及隐式目录预算；支持 ustar、常见 PAX 本地 path/size、GNU 长名、UTF-8、空目录，去除标准前导 `./` 并省略根目录条目。
- 使用锁定的 tar 0.4.46 原始头读取，不在限额前展开 GNU/PAX/sparse 内容；扩展头和根目录计入扫描预算，扩展数据计入字节预算。TAR 头有 checksum，**普通 payload 没有 CRC**，提取成功不能证明其内容未被修改。
- 明确拒绝链接、特殊/稀疏文件（含 GNU、SCHILY、Solaris sparse 扩展），以及当前未实现的全局 path/size、重复 path/size 覆盖等变体。要求完整结束块、仅零尾部，不能将隐藏条目/拼接容器当作成功；这是严格兼容策略，部分标准工具可接受的尾部也会被拒绝。此阶段不读取压缩 TAR（后续 GZIP 支持见下），也不创建新 TAR。
- 有效首部 TAR 优先于 ZIP magic/SFX 探测，防止 PAX 头和 ZIP 尾部混合时隐藏 TAR 内容；无效 TAR checksum 不误拒普通 ZIP/MZ SFX。PAX 关键词长度和控制字符也受检查，避免未知扩展将巨大输入放大到错误消息。
- UI 打开/提取采用通用接口；Web Worker、桌面原子目录发布和 Android SAF 复用同一解码结果。Web/Android 保存原归档按识别出的类型使用 `.zip` 或 `.tar` 与对应 MIME，不重新打包、不会把 TAR 标成 ZIP；创建操作仍只生成 ZIP。
- 本轮验证：core 40 项测试，新增 native、headless UI、worker TAR 流程通过；系统 tar 与 Python tarfile 验证参考样本。浏览器 33 项通过，包含误命名 `.zip` 的 TAR 正确识别、下载字节不变和 OPFS 选中提取。Android 完整 ZIP+TAR smoke 已通过：真实 application/x-tar 保存/取消、字节完全一致及独立 tarfile 校验、真实 SAF 子树/Unicode/二进制/空目录、未选内容缺席、既有输出不变。设备记录 `target/android-smoke/be58e81d781d/console.log`。
- 主机 JNI 检查已覆盖 ZIP/TAR 格式传输、非法代码拒绝和生命周期；格式与结构错误统一使用 archive 文案。Rust 回归/Clippy、格式检查保持通过。iOS 当前配置仍未发现完整 SDK；Docker 指向本地 OrbStack socket，但 daemon 未运行，未据此宣称 Linux 运行验收完成。

- 新增 TAR.GZ / TGZ 内容识别与读取，复用已有纯 Rust flate2/zlib-rs；创建仍只支持 ZIP，不将普通单文件 GZIP 误认为 TAR。多个 gzip 成员作为同一 TAR 字节流连接，支持跨头部/文件边界；整个流的 CRC、ISIZE、可选 FHCRC 和尾部均验证后才返回列表或选中内容。
- 解码前检查 32 MiB 输入上限，输出按总内容与 TAR 头部/对齐预算约束；GZIP 头单个最多 64 KiB、累计 1 MiB，成员最多 4096，不按 ISIZE 分配。压缩比按实际 DEFLATE 字节检查，空成员不增加预算；固定 10 KiB 余量允许标准空 TAR 的结束记录，不能无限放大。严格 TAR 路径/特殊文件/结束块策略保持有效。
- `list_archive_with_format` 一次验证返回格式与列表，避免主机重复解压。Web 打开改由真实 Rust Worker 执行解码，返回有界元数据；主线程不再次探测 GZIP。列表阶段支持 Cancel，错误或取消保留原快照和选择，结束后释放 Worker；文件 picker 读取阶段仍受 nsh 自身限制。
- Web 下载与 Android 保存使用 `Archive.tar.gz` / `application/gzip` 并保留原字节，文件筛选支持 `.gz`、`.tgz`；原 TAR/ZIP 的格式与保存语义保持一致。

- 本轮 Rust workspace 103 次测试执行通过（含低句柄子进程重复），core 55 项；主机/wasm/Android target Clippy 与格式检查通过。浏览器 36 项通过，覆盖真实列表 Worker、TAR.GZ MIME/原字节下载、CRC 失败保留快照、OPFS 子树提取和 Cancel/pagehide 后重试及旧回调隔离。pagehide 仍为合成事件，OPFS 仍不替代外部目录权限验收。主机 Android 检查包括 27 项 JNI、86 项导出策略、13 项资产、11 项有界读取及生命周期，覆盖三种保存格式。

- Android 36 全量 ZIP/TAR/TAR.GZ smoke 通过，记录 `target/android-smoke/016b44ede8fc`；最终 APK 重建后 TAR.GZ 定向复验通过，记录 `target/android-smoke/e4a54df9cfd4`。真实 `application/gzip` 保存/取消、原字节和独立 gzip/tarfile 核对、SAF 子树提取、空目录和未选内容缺席、CRC 拒绝前后快照/选择/既有输出均已检查，PID 稳定且无应用 fatal。专用模拟器测试文件已清理；仍仅覆盖本机 Documents provider，未新增 iOS 或 Windows/Linux 实际运行证据。

- 7z 后端采用固定 `sevenz-turbo = 0.29.5`，关闭默认 features，锁定纯 Rust LZMA/CRC 依赖；最低 Rust 版本提升为 1.97.1。使用其 [ArchiveLimits](https://docs.rs/sevenz-turbo/0.29.5/sevenz_turbo/struct.ArchiveLimits.html) 在头部/名称/对象分配和 decoder 构造前限制资源，不使用默认无限预算构造；字典和模型的估算预算不等于整个应用的 heap 上限。后续升级必须重跑恶意头部、计数、solid 和实际浏览器测试。
- 本阶段读取普通及 solid 7z，支持 COPY/LZMA/LZMA2 与已启用过滤器；复用路径冲突、特殊文件拒绝、选择子目录和原子/授权目录输出。元数据先整体校验，再完整消费选中内容所属 block（包括其中未选成员），等待 block 校验成功后交付结果；未涉及 block 的内容 CRC 不在局部提取时验证，列表不宣称验证 payload。数据块禁用后端会提前返回 EOF 的 CRC 包装，由 core 对实际读取逐成员/逐块计算 CRC，同时保留后端流末尾检查；防止声明前缀 CRC 掩盖 LZMA2 多余输出。
- 只接受完整、头部和 packed ranges 连续的非 SFX 单文件容器；拒绝恢复猜测、附加尾部、加密、anti-item、链接/reparse/特殊类型及未启用 codec。创建仍仅 ZIP，不宣称已完成 7z 加密、分卷、更新或写入。Web/Android 保存使用 `.7z` 和 `application/x-7z-compressed`，保留输入字节；新依赖的许可证和贡献者声明随 Web/Android/iOS 打包脚本复制。

- 7z 完整性边界：校验外层 start/next header、被读取成员和块的已声明内容 CRC；CRC 可缺省，不能作为真实性证明。锁定后端尚不核验 packed-stream CRC，也不对压缩头声明长度之后追加 EOF 探测；不据此宣称覆盖所有畸形编码的完整性。特殊零长度 stream block 在入口拒绝，普通无 stream 空文件/目录继续支持。

- 本轮新增 18 项 7z 核心测试，core 合计 73 项，工作区 125 次执行通过。样本由系统 7z 独立生成，覆盖 COPY/LZMA/LZMA2、solid/non-solid、压缩头、BCJ/BCJ2/Delta、中文/空目录、链接/特殊属性拒绝、头部 CRC/范围/计数/名称/字典限制和选择子树；构造 LZMA2 前缀 CRC 样本确认多余输出不再被 CRC 包装掩盖，未选 solid 内容损坏不会交付部分结果。主机、WASM、Android target Clippy 与格式检查通过。iOS 真机/模拟器、Windows、Linux 的核心交叉编译通过；这些仅为编译证据。

- 最终 Web 优化产物的 37 项浏览器/文件导出测试通过：真实 7z 列表与提取 Worker、内容识别、原字节及 MIME 下载、头 CRC/next-header 失败保留快照，OPFS 输出按系统 7z 样本核对。Android 主机桥接检查覆盖 28 项 JNI 与 ZIP/TAR/TAR.GZ/7Z 保存类型，所有原生命周期/有界 I/O 检查通过。

- Linux 实际运行：启动本机 OrbStack，使用独立 Ubuntu 24.04 arm64 容器、GTK 4.14.5、GStreamer 1.24.2、Rust 1.99.1，源码只读挂载。最终 core/native/worker/UI 共 125 次测试执行通过；真实 GTK 应用在 Xvfb+DBus 下连续两次经文件选择器导入 7z、选择 docs/ 并提取，系统 7z 字节对照、空目录、未选内容缺席和哨兵不变均通过；中文字体和界面截图已检查。证据位于 `target/linux-runtime-20261108-a/final-test.log`、`final-smoke1.log`、`final-smoke2.log`。新增 `bash scripts/smoke_linux.sh target/debug/peazip-ui target/linux-smoke` 和 Linux CI 步骤；远程 CI 尚未运行。该脚本针对 GTK 4.14 英文 X11 固定布局，不代表 Wayland、门户或所有桌面环境；容器已停止，保留可复用缓存。

- Android 最终 APK 7z 真实 DocumentsUI/SAF smoke 通过，记录 `target/android-smoke/a5f82157f576`：打开 6 项、`application/x-7z-compressed` 保存/取消、保存字节与独立系统 7z 校验一致，选中 docs/ 提取 5 项、中文/二进制/空目录正确、未选文件缺席、既有哨兵未改；损坏头 CRC 在写出前拒绝并保留旧快照/选择。PID 稳定无应用 fatal，测试文件已清理，专用模拟器已关闭。

下一步：补充更多归档格式与大文件流式处理，同时继续推进 iOS SDK/文件导出和 Windows/Linux 实际运行；补齐 Web 外部目录、桌面/移动端主动取消与进度、冲突策略及文件管理功能。Web/Android 打开归档仍走 nsh picker，创建使用独立有界入口；两者限制不可混为一谈。六平台实际运行、完整功能与发布验收仍是最终要求。

## 归档完整性测试

- 新增 `test_archive` 和各端 Test archive 操作：检查当前不可变快照中的全部条目，不受选择范围影响，不打开保存/目录对话框，也不写出文件。ZIP/7z 使用固定缓冲读取，7z 完整检查所有 block 与成员 CRC；TAR.GZ 保留已有有界整包解码，避免再复制各文件 payload。
- 报告明确区分条目、文件、实际内容字节和带内容校验和的文件数量。TAR 仅验证结构且覆盖数为零，GZIP 容器 CRC 覆盖全部文件；7z 只有实际存在成员或所属数据块 CRC 的文件计入，空文件不虚增覆盖。不宣称校验和证明真实性，也不扩大已记录的后端完整性保证。
- Web 使用真实独立 Worker，只返回 20 字节有界报告；支持 Cancel/pagehide、重试和旧回调隔离。桌面和 Android 使用后台任务；成功、失败、取消均保留原快照和选择。桌面/移动端此操作的主动取消仍待接入。
- 验证：core 82 项、macOS 工作区 139 次测试执行通过，主机/WASM/Android Clippy 与格式检查通过；16 MiB ZIP/7z 样本的最大单次分配小于 4 MiB，确认未保留整份文件输出。浏览器 41 项通过，包括四格式校验覆盖、未选文件损坏、只读操作和实际 Worker 取消/旧回调。Python 6 项回归通过。
- Android 36 最终 APK 定向 smoke 通过，记录 `target/android-smoke/8204e9cb53b6`：ZIP/TAR/TAR.GZ/7z 报告精确，未选 ZIP 文件 CRC 损坏被拒绝，快照/选择/Downloads/Documents/既有输出保持不变，无系统保存对话框，PID 稳定且无应用 fatal；测试文件已清理，专用模拟器已关闭。
- Linux GTK/X11 测试脚本新增实际点击 Test archive、OCR 核对报告与选择、无输出及哨兵检查，然后继续提取并比较真实字节；连续两轮通过，证据位于 `target/linux-runtime-20261108-a/test-archive-final1.log`、`test-archive-final2.log` 与 `test-archive-final-tests.log`。CI 加入 tesseract-ocr 依赖，仍未运行远程 CI；GTK 固定布局与 X11 验证范围不变。

## 原版视觉基线与归档导航

- 原版基线已采集：官方 PeaZip 11.4.0 Linux ARM64 GTK2 portable，与固定源码版本号一致；下载资产 SHA256 与官方发布值一致。`tests/ui-baseline/peazip-11.4.0-linux-gtk2/` 保留 1024×600、1440×900 的空文件夹、sample.7z 根目录与 docs 子目录共六张真实截图，以及版本、来源、采集步骤和哈希。版本一致不等于证明发布二进制由精确固定 commit 构建；GTK2 原版与 GTK4 新版的平台差异单独记录。
- 归档视图默认按目录层级浏览，支持隐式父目录、Back/Forward/Up/Archive root、路径输入和 Go、目录优先的名称/大小双向排序；All entries 可切回全部真实条目的平铺视图。历史最多 256 项，成功打开/创建重置历史与选择，重新打开相同归档也清除旧的无效路径草稿和错误。
- 宽屏侧栏和主列表均按当前层级生成并虚拟化；窄屏隐藏侧栏与 Type 列，保留导航和操作。复用原版 main/main-dark 的操作与导航 PNG 图标（未改像素，嵌入 data URI），普通文件使用简单纸张轮廓占位；原 `[ ]` 文本选择标记替换为方框、部分选择指示及行高亮。名称、类型和归档路径限制单行，完整路径仍作为控件可访问名称；长中文文件名及扩展名的窄屏实际渲染测试通过，无页面横向溢出。
- 选择语义反映实际提取范围：选中父目录时后代显示已选，取消子项会展开祖先选择为剩余真实文件/空目录，避免 core 再次包含被取消内容。隐式目录只映射到真实后代条目，不能向 core 传递不存在的路径。新增资源回归确认深层路径不会一次生成数万个目录节点。
- 本轮 macOS 工作区 152 次测试执行通过，UI 包含 11 项模型测试和 17 项 headless 流程；父目录选中后取消子文件已验证实际落盘排除。浏览器 56 项通过，覆盖导航、排序、非法路径、重开状态、实际 OPFS 排除输出，以及 1024×600、1440×900、360×800 下亮暗主题。截图为 `target/web-folder-*.png`；生产源码摘要和构建命令记录在 `target/ui-navigation-evidence.json`。
- Linux 最终源码哈希核对一致，152 次测试执行与两轮真实 GTK 导航/Test/解压通过；六张新版对照截图、验证元数据在 `target/linux-ui-rewrite/`。原版与新版同尺寸/同 sample.7z 场景已人工对照；像素差异只记录现状，不充当自动通过阈值。空状态原版是空文件夹，新版是未打开归档，不能据此宣称本地文件系统视图已迁移。脚本改用 OCR 定位主要控件，保留真实文件选择与字节对照；只覆盖已记录的 GTK/X11 环境。
- 路径编辑使用字面输入（不自动大写或纠正）；合法 Enter/Go、历史导航与重开归档后显式结束编辑，避免 Android 首次目录点击只失焦而未选择。非法 Enter 保留编辑焦点，便于修正。
- Android 最终 APK 的完整导航 smoke 退出码 0，记录 `target/android-smoke/a724445945d9`；真实 DocumentsUI/SAF 验证隐式目录仅导出实际后代、父目录选中后取消子文件确实不输出该文件、空目录/其他输出/哨兵正确，第一次点击生效，路径 Enter 成功失焦、非法输入保留焦点，重开与视图偏好正确。独立输入复验记录 `target/android-smoke/fd0c358f069f`。新增可重复运行的 `uv run python scripts/smoke_android_navigation.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk`；测试文件清理、PID 稳定无应用 fatal，专用模拟器已关闭。
- 阶段 2a 仍未完成：原版完整菜单、文件系统/书签/历史导航、双击与快捷键、列宽调整及完整属性列、创建/解压选项对话框、跨平台窗口/主题和 iOS/Windows 实际运行均需继续验收。此提交是可用的归档导航单元，不将它或六张截图等同于完整 PeaZip 界面迁移。

## 系统菜单与共享命令

- File、Edit、Browser、Layout、Tools、Help 接入已有归档操作、选择、导航与排序。macOS/Linux/Windows 代码使用 nsh 原生应用菜单，macOS 保留 PeaZip 应用菜单及系统 Hide/Quit roles；Web 与移动端使用同一命令集合的弹出菜单/底部菜单。未接入的 Organize、Options 及原版其余命令仍待迁移，不以空操作伪装可用。
- 菜单、工具栏、导航按钮与快捷键共享实时 enabled/checked 与实际调用；执行时再次检查状态，旧菜单不能绕过 busy 或空快照限制。菜单启动的任务归属 App owner，不随 popup 销毁而取消。新增 Close archive，仅清除当前快照、选择和导航，不改源文件；再次打开从根目录开始，视图偏好保留。
- 桌面使用 mod+O/N/W/L、mod+Enter；选择与导航还支持 mod+A、mod+Shift+A、Alt+Left/Right/Up（mod 按平台映射 Command/Control）。编辑敏感操作只在非文本编辑状态响应，Web 不注册浏览器保留的打开/新建/关闭/地址栏组合键。原生 accelerator 仅给 Open 和 FocusPath，避免 GTK/AppKit/Win32 忽略 NotEditing 而抢走文本全选；也保证菜单栏或空白区焦点下能打开归档。
- 原生归档菜单动作完成后回到列表焦点，About 结束后再回焦；FocusPath 仍聚焦地址栏。实际修复了 GTK Edit/Clear 后焦点落入路径框、下一次 Ctrl+A 只选中文字的行为。Web 菜单采用独立菜单按钮组，修复通用 toolbar 与菜单同时处理 Enter 的重复切换；About 开启前记录有效触发器，关闭后恢复焦点。快捷键提示显示 Cmd/Ctrl 等用户可识别名称。
- 验证：工作区 159 次测试执行通过；新增实际 App 的原生菜单 mount/事件分派测试，检查菜单分组、disabled/checked、编辑敏感项无 native accelerator，以及真实打开/关闭与状态更新。headless 覆盖快捷键实际打开和提取、路径编辑不误选、关闭与重开。主机/WASM/Android target Clippy、格式检查通过。
- 浏览器 63 项通过：菜单触发异步归档任务、忙碌门禁、菜单取消真实 Worker、同一份快照/选择、路径编辑快捷键隔离、键盘打开菜单和焦点恢复；桌面与触屏亮暗主题截图已检查，触屏 Go to path 在 sheet 关闭两帧后仍能直接输入。源码/构建证据记录在 `target/menu-evidence.json`。
- Linux 当前源码的原生 GTK 菜单两轮完整流程与后续焦点补充流程通过，包含 File/Open、Edit/Select/Clear、Browser/Layout、Tools/Test/Extract、File/Close、About，以及不额外点击内容区的 Ctrl+A 和排序后的 Alt 导航。16 项焦点探针通过；源码哈希和截图在 `target/linux-menu-native/verification.json`、`images.json`。保留 Xvfb DRI3 与 GTK Separator 尺寸 warning，不宣称日志完全无警告；没有 panic/critical。环境仍限定为已记录的 GTK/X11。
- Android 最终 APK 菜单 smoke 退出码 0，记录 `target/android-smoke/00d122abec9c`：菜单 disabled/checked、File 打开/保存取消/关闭、Browser 历史与路径聚焦/Enter 失焦、Edit 选择、Layout 视图排序、Tools 只读 Test、Back/外部点击关闭、About 均通过；源文件/选择及取消状态按场景核对，PID 稳定无应用 fatal。可复现命令为 `uv run python scripts/smoke_android_menus.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk`。测试文件已清理，专用模拟器已关闭。
- 本轮重新尝试 iOS UI 检查仍因缺少 iphonesimulator SDK 在 ring 构建阶段失败；Windows UI 交叉检查缺少 x86_64-w64-mingw32-gcc。两者不计作 UI 构建或运行完成；既有 CI 配置尚未获得远程执行证据。完整菜单迁移、原版操作对话框、本地文件管理及其余阶段 2a 验收继续保留为未完成。

## Windows GNU 可重复构建

- 新增 `uv run python scripts/build_windows_gnu.py`，默认 locked release；要求显式提供官方 x64 WebView2 loader DLL 及许可证/NOTICE，校验固定 SHA256 后生成 GNU import archive，隔离目标 flags，不修改 nsh 或全局 Cargo 配置。产物附 loader、许可证、工具版本、命令、依赖清单和 SHA256SUMS；WebView2 Runtime 仍须由运行环境提供。
- Ubuntu 24.04 arm64 + MinGW GCC/G++ 13-posix + Rust 1.99.1 已完成当前 UI 的 Windows GNU check 与 release build。`target/windows-build/release/peazip-ui.exe` 为 56,953,497 字节 x86-64 Windows GUI PE；PE 解析结果与独立 objdump 对照一致，错误 loader 在 Cargo/输出目录创建前拒绝。最新文件浏览焦点修复已包含，源哈希与验证记录在同目录。仅为构建/静态检查证据，未验证 Windows 实际运行，未生成安装器。

## 桌面文件系统浏览

- macOS/Linux/Windows 新增原生文件夹选择、即时目录列表、名称/类型/大小列、排序、稳定条目选择、路径输入、上级/根目录、刷新，以及统一的文件夹/归档内部前进后退历史（最多 256 项）。从列表打开支持的归档；当前选择创建 ZIP 只请求保存位置，复用安全递归和原子不覆盖发布。关闭归档返回并刷新父目录，普通加载失败保留旧视图和选择。
- 原始 OsString 名称与显示转义分离，所有行操作基于持有的目录 capability 和快照 ID；链接/reparse/特殊文件只列出、不跟随或打包。默认单目录最多 16384 项、单名称 1024 字节、累计名称 16 MiB，超限整体拒绝，扫描失败/消失条目也计数；不为每行持有文件句柄。显式路径导航先打开再规范化并比对目录句柄身份，修复含 `..` 路径的上级导航；显式路径的父组件仍是可信环境路径，不宣称全路径沙箱。
- 文件夹与归档选择/命令资格分离，切回文件夹清空归档快照。Open folder 使用 mod+Shift+O，Open selected 使用 mod+Shift+Enter，刷新 F5；文本编辑敏感快捷键继续受 NotEditing 门禁。共享命令和异步操作归属 App owner；文件列表挂载后再恢复焦点，修复 GTK 关闭归档后 F5/Ctrl+A 不生效。成功输入当前归档路径也明确退出编辑焦点。
- 验证：主机工作区 191 次测试执行通过，新增 10 项实际 App 文件浏览集成测试；最终焦点变更重跑 UI/文件浏览及 Clippy 通过。主机/WASM/Android target Clippy 和格式检查通过；Web 优化构建及 63 项浏览器回归通过，Android APK 构建和 28 项 JNI 桥接检查通过。Linux 原生 browse 13 项及低句柄子进程通过，实际非 UTF-8 文件名分支已执行，记录 `target/linux-filesystem/native-browse-final.log`。
- 真实 GTK/X11 流程通过：文件夹选择、路径/根目录/历史、Unicode 文件及空目录选择创建 ZIP（系统 7z 独立提取逐字节对照）、7z/TAR 打开与关闭、外部新增文件后直接 F5 刷新并清除选择、非法路径/归档/外部链接拒绝，以及整个目录被替换后仍通过原 capability 读取。最终应用源哈希、截图与流程记录在 `target/linux-filesystem/focus-fixed-source.json`、`focus-fixed/run-29HUwD`、`focus-fixed.log`；1024×600 与 1440×900 截图已目视核查。新增 `scripts/smoke_linux_filesystem.sh` 并接入 Linux CI；最终脚本完整复验通过，记录 `target/linux-filesystem/ci-final.log`，`ci-source.json` 中脚本和应用源哈希与待提交内容一致。远程 CI 尚未运行。
- 当前 APK Android 菜单回归通过，证据 `target/android-smoke/78157b7fb499`、`menus-owner-0bb5d9c6c107.log`；安装 APK 哈希与本地一致，PID 稳定无应用 fatal。Android/Web 只回归原归档流程，本轮没有开放桌面文件系统浏览能力；iOS SDK/运行验证和 Windows 实际运行仍未完成。
- 阶段 2a 继续未完成：完整文件管理、书签、原版列/布局/双击行为及选项对话框、所有平台视觉和运行验收仍需推进。本轮是文件浏览功能单元，不等同于 PeaZip 完整界面迁移。汇总证据位于 `target/filesystem-evidence.json`。

## 创建归档选项（UI-CREATE-01）

- Create ZIP / Create folder ZIP 先显示创建表单，展示 ZIP 格式、输入来源和压缩设置；Continue 后才调用既有文件/目录及保存选择器。文件浏览已有选择时继续使用当前目录 capability 和原始条目 ID，不重新请求输入选择。Cancel、Escape、Android Back 不启动创建或保存；表单期间旧菜单、工具栏和快捷键同样受门禁限制。确认操作归属 App owner，不随表单销毁取消。
- 核心新增 `ZipOptions` / `ZipCompression` 与 `create_zip_with_options`，native 路径/快照创建贯穿参数，旧 API 保留默认行为。Automatic 保留原有 Deflate 超压缩比时回退 Store 的策略；Stored 真正写入不压缩数据；显式 Deflate 接受 1–9，非法级别立即拒绝，超安全压缩比报错且不回退。目录使用 Stored。既有路径、大小、计数和原子不覆盖发布策略不变。
- Web 新增独立版本化创建请求，保留旧入口兼容；JS 在打开 picker / 读取文件前验证参数，Rust 再验证版本与参数并限制请求长度。选项穿过实际 WASM Worker；取消、进度、旧回调隔离保持有效。Android 选项传入后台 Rust 创建，沿用 SAF 保存。确认过的选项在当前运行会话内保留供错误后调整重试，取消不保存草稿；尚未实现跨启动配置持久化。
- 原版参考新增 `tests/ui-baseline/peazip-11.4.0-linux-gtk2/*-create-zip-normal.png`，复用官方 11.4.0 GTK2 二进制，采集 1024×600/1440×900 的 ZIP/Normal/New archive/Single volume 面板，验证 Cancel 后输入不变且无输出。来源、步骤、哈希与配置记录在该目录 `provenance.json`，未断言发行二进制来自精确源码 commit。
- 原版行为差异明确保留：`peach.pas:23294` 默认 Fast/Deflate，`peach.pas:31916` 的 0/1/3/5/7/9 预设还联动 Word/Passes；当前数字级别来自 Rust ZIP 后端，不冒称完整复刻这些预设。Automatic 是当前兼容选项。原版全窗口输入列表/编辑、输出参数、多格式、加密、分卷和 Advanced/Console 等仍待迁移，本表单不代表完整创建面板或阶段 2a 完成。
- 主机工作区 207 次测试执行通过，包含真实压缩方法与不同级别效果、非法参数、旧 API 兼容、no-overwrite、modal 取消/输入资格/会话参数和错误重试；主机/WASM/Android target Clippy 通过。Web 完整 71 项通过；最终布局版本另跑 5 项创建相关浏览器流程通过，包含真实 Worker 的 Stored/Deflate1/9 产物及解压字节、ratio 失败保留原快照与选择后改 Store 重试，以及 1024/1440 桌面和 360/430 触屏亮暗主题截图。证据 `target/create-options-workspace-tests.log`、`create-options-web-tests.log`、`create-dialog-web-final.log`。
- 表单采用 nsh 原生 modal 与可访问控件，窄屏内容可滚动；为软键盘增加底部空间，使数字输入时确认/取消仍可操作。锁定 nsh 的 AccessibilityRole 尚无 Dialog，因此内容使用带名称的 Group；Web 的完整 dialog 角色语义仍待依赖能力补齐，不以当前截图或控件测试宣称完整屏幕阅读器验收。

- 最终 GTK 创建流程通过，记录 `target/linux-create-options/run3.log`；`run3-source.json` 中应用与脚本哈希已核对一致。原版归档快照/选择在 Cancel/Escape 与 ratio 失败后保持，输出目录哨兵不变；独立 7z/ZIP 检查确认 Store 方法 0、Deflate 方法 8，级别 1/9 的样本压缩后分别 4663/2936 字节，错误后可在同目标路径改用 Store 成功。新增 `scripts/smoke_linux_create_options.sh` 并接入 Linux CI，远程 CI 未运行。
- 最终 Windows GNU optimized release/check 通过，`target/windows-build/create-options` 包含 57,103,499 字节 GUI PE、官方 loader 与许可证/哈希；应用源哈希与待提交内容一致。该结果仍只证明构建与静态导入检查，不证明 Windows 实际运行。iOS 仍缺完整 SDK，未增加其构建或运行声明。

- 最终 APK Android 36 实机 smoke 完整通过，证据 `target/android-smoke/89665dcba2a2` 与 `create-options-run.log`；安装/测试 APK 与最终本地产物 SHA256 一致。真实 DocumentsUI/SAF 保存后以 Python zipfile 与 raw DEFLATE 独立检查名称、方法、CRC 和全部字节；647168 字节样本在 level1/9 下分别为 25915/20680 字节，Stored 保留原长度。Cancel/Back 无文件选择器启动、原快照/选择/Downloads 不变，ratio 失败后再次保存的旧快照字节完全一致，改用 Store 重试成功。最终截图确认 IME 打开时 Continue/Cancel 可见且真实点击成功，PID 稳定无应用 fatal。可复现脚本 `uv run python scripts/smoke_android_create_options.py --serial SERIAL --apk APK`；仍只覆盖专用 API36 模拟器与本机 Documents provider。
- 最终创建表单 headless 5 项复验通过；各构建、截图与测试汇总源哈希在 `target/create-options-evidence.json`。原版完整创建面板、更多格式/加密/分卷、流式大文件、全部平台实际运行与发布门槛仍保留为未完成。

## 解压范围与路径选项（UI-EXTRACT-01）

- 统一入口改为 Extract…：有归档条目即可打开表单，无选择时默认 All entries，有选择时默认 Selected entries；零选择的 Selected 控件明确禁用。表单打开时固定不可变字节快照、全部路径和所选路径，All 不改写原选择。所有主窗口命令在表单期间受门禁约束，确认后的任务归属 App owner；Cancel/Escape/Android Back 不启动解压选择器。已确认的路径方式在当前会话保留，取消不保存草稿。
- 核心新增 `ExtractOptions { paths: ExtractPaths::Full | FilesOnly }` 与 `extract_selected_archive_with_options`，native 增加对应发布 API；旧 API 默认 Full 保持兼容。FilesOnly 必须先完成原归档路径、类型、资源预算及所需内容 CRC 校验，再丢弃目录项并将文件映射为 basename；复用大小写归一与冲突检测，重名整体拒绝，不自动覆盖/改名。空目录不保留，零字节文件保留，只有目录时返回明确错误且不发布空输出。
- Web 使用独立版本化提取协议，保留旧入口；JS 在目录 picker 前验证路径模式，Worker 在 WASM 初始化前验证版本/路径模式，Rust 再重验，输出协议与取消/进度/旧任务隔离保持不变。Android 在 Rust 后台完成解码与映射后再请求 SAF；native 保持新目录的临时写入与不覆盖发布。Web/SAF 继续沿用既有 provider/并发边界，不宣称跨进程原子 no-replace。
- 新增四格式共 16 个正常层级、重名、目录-only、恶意路径样本，`uv run python crates/peazip-core/tests/fixtures/generate_extract.py` 用标准库及系统 7z 生成，7z 恶意路径仅修改等长 UTF-16 名称并重算头 CRC，不在磁盘建立越界路径。测试覆盖 ZIP/TAR/TAR.GZ/7z 的路径保留、flatten、大小写重名、未选恶意路径不能被洗白、CRC 失败、资源限额和输出不覆盖。
- 原版提取面板新增四张普通/without-path 的 1024×600/1440×900 参考图，复用官方 11.4.0 GTK2 二进制；Cancel 后归档与预置输出哨兵不变。来源/步骤/哈希见 `tests/ui-baseline/peazip-11.4.0-linux-gtk2/provenance.json`，未断言发行二进制来自精确源码 commit。
- 固定源码 `367c682…` 的 `peach.pas:60478` 在无选择时采用 displayed 范围，该阶段先提供 All/Selected；displayed 在后续 UI-EXTRACT-02 补齐，高级过滤仍待迁移。`peach.pas:51675` 将 Extract/without-path 映射为 7z x/e；当前 Full paths 指新目标内的安全相对目录树，不是原版高级 Full (-spf2) 或 Absolute (-spf) 模式。`peach.pas:51630` 的跳过/重命名/覆盖/询问等冲突策略、已有目录合并、多归档输入、密码、Advanced/Console 与完整面板布局仍待迁移，不将本功能单元计作阶段 2a 完成。
- 验证：主机工作区 226 次测试执行、完整 Web 83 项、主机/WASM/Android target Clippy 与格式检查通过；最终禁用态样式重建后，6 项提取浏览器流程再次通过。真实 WASM/OPFS 检查 All 无选择、Selected、Full/FilesOnly、空目录与零字节文件、重名及目录-only 错误无输出、Full 重试和 user activation；OPFS 仍不替代外部目录授权验收。1024/1440 桌面及 360/430 触屏亮暗主题截图已检查，沿用现有 nsh Group/modal 的可访问性能力边界。
- 最终 Linux GTK/X11 新选项流程完整通过，`target/linux-extract-options/final.log` 覆盖 All/Selected × Full/FilesOnly 的实际 7z 数据、空目录策略、同名拒绝、原快照/选择/输出不变和同目标 Full 重试；ZIP 碰撞样本逐文件核对。原菜单/快捷键/导航/Test/Extract/Close 兼容 smoke 同样通过，记录 `archive-compat.log`。最终脚本和应用源哈希已核对一致，脚本接入 Linux CI；远程 CI 尚未执行。
- 最终 APK 在 Android 36 专用模拟器完整通过真实 DocumentsUI/SAF ZIP 流程，证据 `target/android-smoke/cdd3cd412688`、`extract-options-run.log`；tested.apk 与本地产物 SHA256 一致。Cancel/Back 无 picker，全部/所选/无目录输出包含 Unicode、二进制与空文件，逐路径/字节核对且既有输出不变；重名在 SAF 前拒绝，Full 重试成功，PID 稳定无应用 fatal。仅覆盖本机 Documents provider，未扩展到所有 provider 或设备版本。
- Windows GNU 最终 check/release 与 PE/loader/hash 检查通过，记录 `target/windows-build/extract-options`，生产源哈希与待提交内容一致；未增加 Windows 实际运行声明，iOS 本轮未新增构建/运行证据。汇总源哈希与测试记录在 `target/extract-options-evidence.json`。

## 当前显示范围解压（UI-EXTRACT-02）

- 补齐固定原版 `peach.pas:60478` 的无选择 displayed 默认行为：有选择仍默认 Selected，无选择默认 Displayed，All 始终为明确可选范围。表单显示当前归档目录或 All entries view，以及所选/显示范围/整个归档的条目数。
- Displayed 来自当前完整逻辑列表，不是虚拟滚动窗口中的少数已绘制行；目录覆盖后代，隐式目录只展开为真实条目，排序不改变范围。文件夹模式不包含同级或近似前缀目录，平铺模式覆盖整个归档；目标仍保留归档内完整相对路径，FilesOnly 策略沿用既有规则。
- 表单打开时将 displayed 路径集和位置说明与字节快照一起冻结。操作不改写原选择；无选择的空目录默认 Displayed 无可提取内容，Continue 禁用，不悄悄回退 All。用户可取消，或明确改为 All 后继续。
- 工作区 231 次测试执行、Web 84 项、主机/WASM/Android target Clippy 与格式检查通过。新增范围测试覆盖目录边界、四种排序、隐式目录、空视图、平铺范围、删除源文件后仍使用快照、选择保持和显式 All；实际 Web Worker/OPFS 验证子目录、隐式子目录与空视图切换，桌面/触屏亮暗主题截图已检查。
- Linux 最终 GTK 流程通过，记录 `target/linux-displayed-extract/final.log`；真实 7z 字节核对确认 docs 范围不包含 other.txt，空视图阻止确认、显式 All 可用，平铺 Displayed 包含全归档；原路径方式和同名冲突重试继续通过。应用/脚本哈希与待提交内容一致。
- Android 36 本机 Documents provider 实机完整通过，记录 `target/android-smoke/1dca19266819`、`displayed-extract-options-run.log`；docs Displayed 导出 4 个真实后代，平铺 Displayed 导出全归档 7 项，空目录取消/显式 All 正常，原选择及既有输出保持。最终 tested.apk 与本地产物哈希一致，PID 稳定无应用 fatal；专用测试文件和模拟器已清理。
- Windows GNU check/release 与 PE/loader/hash 检查通过，产物 `target/windows-build/displayed-scope`；未验证 Windows 运行，iOS 本轮未增加验证。证据汇总 `target/displayed-evidence.json`。高级过滤、冲突处理和原版完整提取面板等既有缺口继续保留，未将这一交互补齐视为整个迁移完成。

## 当前视图反选（UI-SELECT-01）

- Edit 菜单与归档/桌面文件夹选择工具栏新增 Invert selection，共用命令资格和处理器；空视图、busy 与操作表单期间禁用。桌面增加 mod+I，使用 NotEditing 且不注册 native accelerator，编辑地址栏时不会反选文件；Web/移动端保留菜单与按钮入口。
- 归档按当前完整逻辑列表所覆盖的有效文件和空目录取补集，视图外有效选择保持；隐式目录、平铺视图中重叠父子行和部分选中目录不会通过父目录重新包含已取消的子项。连续两次恢复有效内容集合。与既有取消子项行为相同，内部父目录缩写可能展开为叶子路径，因此不承诺恢复原缩写或原路径计数表示。
- 桌面文件夹按当前不可变目录快照的稳定条目 ID 取补集，排序不改选择身份；操作本身不读写文件。新增端到端测试实际创建 ZIP 和提取 TAR 核对字节，检查目录外选择、部分目录、双次反选、输入焦点、modal/busy 与空视图门禁。
- 原版对应 `peach.lfm:16949` 的 Edit/Invert selection 和 `peach.pas:33909` 的当前列表 0/1 取反。本实现对已有层级混合选择使用有效后代补集；原版创建/解压输入列表内的反选仍随完整操作面板迁移，当前表单期间不会误操作主列表。
- 验证：工作区 238 次测试执行、Web 86 项、主机/WASM/Android target Clippy 与格式检查通过。浏览器以真实 Worker/OPFS 验证子目录反选保留外部选择、父子重叠不重新引入取消项及实际输出字节；菜单/工具栏与禁用资格一致。原生菜单测试确认新命令动态资格和编辑敏感项无系统 accelerator。
- Linux 最终 GTK smoke 完整通过，`target/linux-invert-selection/run3.log` / `verification.json` 验证 Edit 反选、Ctrl+I 有效集合恢复、地址栏编辑时不反选、源归档 SHA 不变，原菜单/导航/Test/解压与哨兵继续通过。选择计数 OCR 改为沿用窄区域读取，保留实际 checkbox 像素与真实输出检查。源码/脚本哈希与待提交内容一致。
- Android 菜单 smoke 完整通过，记录 `target/android-smoke/a88448c6afe5`、`invert-menu-run.log`；全范围 none/all/none、子目录部分选择补集、隐式 nested 文件状态、源字节/快照/Downloads 不变均检查，PID 稳定无应用 fatal，tested.apk 与本地产物一致。测试文件和专用模拟器已清理；仍仅覆盖本机 API36 Documents provider。
- Windows GNU check/release、PE/loader/hash 验证通过，产物 `target/windows-build/invert-selection`；未增加 Windows 运行或 iOS 验证声明。证据汇总 `target/invert-evidence.json`。范围选择、键盘列表导航、右键/拖放及完整原版选择面板等缺口继续保留。
