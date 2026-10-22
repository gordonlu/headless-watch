# Headless Watch — 产品需求文档（PRD v1.0）

> **定位**：让 AI 启动的无头浏览器可见、可追踪、可控。  
> **产品标语**：See the invisible browsers.  
> **阶段**：正式版规划；以已运行的 Python MVP 为基线。  
> **目标平台**：Ubuntu 26 / Linux（Wayland、X11 均支持），单用户、本地优先。  
> **拟定正式技术栈**：Rust + Tokio/Axum + SQLite + Vue 3/TypeScript/Vite；无需 Electron、Docker 或云端。

---

## 0. 执行摘要与决策

Headless Watch 不是通用的 `top` 替代品，也不是另一个 Playwright 浏览器启动器，而是**针对已存在的本机自动化浏览器的独立观测与安全清理工具**。

核心承诺：

1. **零侵入自动发现**：不修改 OpenCode、Codex、Playwright 或 Puppeteer，也能发现其启动的 Chrome/Chromium Headless。
2. **资源数字可信**：CPU 同时显示单核基准百分比、实际核心使用量、整机占比；内存显示口径和准确度等级。
3. **来源可追溯**：尽早记录父链、启动参数、进程标识、cgroup；在父进程退出后仍保留当时观察到的来源信息。
4. **残留不等于违规**：区分进程已被重新收养、疑似残留与未知来源，避免把正常长期任务自动判为垃圾。
5. **安全优先**：观测默认只读；结束进程需要明确确认；截图/网页信息默认按需获取；永不静默开启 Chrome 调试端口。
6. **用户可理解**：2 分钟之内弄清楚“谁启动、用了多少资源、何时开始、是否还在运行、怎么结束”。

**最终产品版本划分**：

- **v0.2（修正基础）**：把 MVP 的指标、身份校验、错误处理、首次体验做正确。
- **v0.5（可日用版）**：正式架构、持久历史、进程来源、低开销扫描、系统通知、完整新版 UI。
- **v1.0（开源发布版）**：残留检测可信度、清理保护、Linux 包装与开机启动、稳定性测试和文档齐备。
- **Post-v1（探索）**：受管理的 Agent 启动模式、实时页面预览、进阶进程事件追踪、Windows 等。

**明确不做**：首版不做云账号、多人权限、团队管理、Docker 集群部署、自动开启浏览器调试端口、默认录屏、全部网络流量监视、无限期的原始指标存档。

---

## 1. 现有 MVP 基线与技术债

仓库基线是 `headless_watch.py`（约 480 行）+ `install.sh` + `README.md`。

### 1.1 已有功能（应继承而非重造）

- 每秒扫描 `/proc`，识别 Chrome/Chromium/headless shell；根据父子树聚合成会话。
- 汇总每会话 CPU、RSS、进程数、启动时长、启动命令、父链及简单来源标签。
- 页面每 2 秒刷新；基本 CPU 历史折线图；新启动与高 CPU 桌面通知。
- 保存少量内存内事件；监测 `--remote-debugging-port` / `DevToolsActivePort`，读取标签页。
- 若 CDP 可访问，按需截图；Dashboard 中可选择 SIGTERM/SIGKILL。
- 本机 HTTP 端口 `127.0.0.1:8765`；systemd 用户服务安装脚本。

### 1.2 必须修复的缺陷

| 编号 | 当前限制 | 修复要求 |
|---|---|---|
| D-01 | 顶部 CPU 百分比是单核口径，容易误解 | 同时显示 `1.05 cores / 105% of one core / 6.6% of machine (16 CPU)`；核心数动态获取 |
| D-02 | 聚合 CPU 用整棵树累计 jiffies 做差，新子进程进入树时可能造成瞬时跳值 | 改为**逐进程 PID + starttime** 单独计算 delta，然后聚合；新进程首帧不计历史 CPU |
| D-03 | RSS 直接求和可能重复计算共享页 | 首屏标注 `ΣRSS（上限型估计）`，可用时按低频计算 `ΣPSS` 并显示采样时间 |
| D-04 | 事件保存在内存，重启即丢 | SQLite 持久化与自动保留期 |
| D-05 | 原始来源父链在进程死亡后无法重新推断 | 检测时立即持久化父链快照、cgroup、相关时间戳；不假称事后百分百可还原 |
| D-06 | `PID 1`/变更 PPID 被直接提示 `Possible orphan`，易误报 | 事实状态 + 来源证据 + 风险评分分开显示 |
| D-07 | 页面全量重建 DOM / 每 2 秒轮询 | Vue 组件粒度更新 + SSE/WebSocket 推送；离线自动重连 |
| D-08 | 仅 Chrome、Chromium 基础识别 | Linux 上识别常见 Playwright bundled Chromium、`chrome-headless-shell`、Microsoft Edge headless；普通有界面浏览器不计入 |
| D-09 | Web 鉴权与敏感信息遮蔽还需加强 | Token、Origin/Host 验证、路径检查、日志脱敏、CSP、自检和安全测试 |
| D-10 | 清理时只看当前子孙，已脱离进程树的同源进程可能遗漏 | 只对可验证的浏览器进程执行动作；清理结果报告 `已结束/仍存活/未授权/已脱离`，不做危险的模糊批量杀进程 |

---

## 2. 目标用户与场景

**主要用户**：在 Linux 桌面运行 Coding Agent 的开发者：OpenCode、Codex、Claude Code、Playwright MCP、Puppeteer、浏览器测试工具的用户。

**使用场景 A — 发现意外浏览器**：用户未启动显式窗口，但系统有无头 Chrome 占满一个核心。Dashboard 自动显示该浏览器、所属工具的证据、活跃时间与占用。

**使用场景 B — Agent 退出但浏览器仍存活**：Headless Watch 之前已经在后台运行。它记录了 Chrome 的启动链。OpenCode 退出后，Chrome 仍存活，界面出现“启动者已退出，浏览器仍运行”的告警，用户选择安全停止。

**使用场景 C — 浏览器正常负载**：Playwright 测试同时创建多个浏览器。用户看到会话分组和总 CPU，能够分辨正常并行任务与疑似失控。

**使用场景 D — 需要看浏览器做什么**：有 CDP 时显示标题、URL 和按需截图；没有 CDP 时仍能完整看到进程层指标，并明确告知不可预览的原因。

**使用场景 E — 事后排查**：次日打开工具，回顾浏览器启动/退出时间、峰值 CPU、持续时长、疑似残留、自己执行过的结束操作。

---

## 3. 范围与优先级

| 需求 | v0.2 | v0.5 | v1.0 | Post-v1 |
|---|:---:|:---:|:---:|:---:|
| 自动发现无头 Chrome/Chromium | ✓ | ✓ | ✓ | ✓ |
| 准确的 CPU 核心数与整机占比 | ✓ | ✓ | ✓ | ✓ |
| 会话列表与 CPU/内存趋势 | ✓ | ✓ | ✓ | ✓ |
| 浏览器进程树与启动命令 | ✓ | ✓ | ✓ | ✓ |
| 新版深色 Dashboard | 设计定稿 | ✓ | ✓ | ✓ |
| 来源快照持久化与历史搜索 | — | ✓ | ✓ | ✓ |
| 浏览器生命周期与孤儿风险识别 | 基础 | ✓ | 完善 | ✓ |
| 桌面通知、静音与告警规则 | 基础 | ✓ | ✓ | ✓ |
| 安全停止、强制结束、操作记录 | 基础 | ✓ | ✓ | ✓ |
| 已有 CDP 的标签列表/按需截图 | ✓ | ✓ | ✓ | ✓ |
| 托盘/快速显示 Dashboard | — | 可选 | ✓ | ✓ |
| 安全的受管理 OpenCode 启动包装 | — | — | 可选实验 | ✓ |
| CDP 连续实时画面 / 鼠标键盘接管 | — | — | — | 探索 |
| eBPF/进程事件增强捕获 | — | — | — | 可选特权模块 |
| Windows / macOS | — | — | — | 另行规划 |

> 原则：v1.0 **默认不接管 Agent、不拦截系统调用、不自动杀进程**；核心是看清楚，并且能安全地做决定。

---

## 4. 功能需求（FR）

### FR-01 自动进程发现与身份识别（P0）

- 每 `1s` 采集一次；可配置 `0.5–10s`，默认 `1s`。
- Linux `/proc` 检测普通 Chrome/Chromium、headless shell、Edge headless 与 Playwright 下载的 Chromium 可执行文件。
- 结合 exe basename、命令行 `--headless`、进程角色 `--type=` 等多证据识别；普通图形浏览器不得误报为无头实例。
- 每个会话身份为 `boot_id + root_pid + root_start_ticks`，避免 PID 复用导致串会话。
- Chrome 子进程按父子链关联；保留 Renderer、GPU、Utility 等角色与 PID 列表。对后来被重新收养的子进程保留原始历史关系，但仅凭历史关联**不得直接扩大 kill 范围**。
- 后端启动时可发现已经在运行的浏览器，标记为 `observed_after_start`，不将未知的原始来源伪装成已确认。
- UI 中呈现 `Active / Exited / Unknown` 生命周期状态，扫描暂时失败时显示 `Stale` 而非立即退出。

### FR-02 资源指标与口径（P0）

每会话必须展示：

| 指标 | 定义 | 界面展示 |
|---|---|---|
| `cpu_cores` | 每个相关进程两次采样间 `Δ(utime+stime)/Δwalltime` 的和 | `1.05 cores` |
| `cpu_single_core_pct` | `cpu_cores × 100` | `105% · 1 core = 100%` |
| `cpu_machine_pct` | `cpu_cores / online_logical_cpus × 100` | `6.6% of machine · 16 threads` |
| `rss_sum` | 所有受归属进程 RSS 的算术和，可能重复共享内存 | `ΣRSS 1.2 GB · estimated` |
| `pss_sum` | 可访问 `/proc/PID/smaps_rollup` 时，低频聚合 PSS | `PSS 845 MB · sampled 12s ago` |
| `process_count` | 会话当前确认的进程数量 | `7 processes` |
| `lifetime` | 根进程从启动至现在的时长 | `00:14:32` |
| `cpu_peak` | 已观察区间峰值 | `Peak 2.38 cores` |

- **所有 CPU 数字必须解释分母**；机器线程数按实际 online CPUs 获取，不写死 16。
- 图表纵轴可切换 `cores` / `% machine`，默认 `cores`；超过一个核心不标红，告警依据可配置。
- 逐 PID 计数 CPU，防止新子进程加入后用过去积累的 ticks 产生跳值；CPU 首次采样标记 `Calculating`。
- PSS 采样默认 `15s`、失败回退 RSS，且在 UI 标明估计值与时效；不可为 PSS 扫描拖慢 1 秒快速采集。
- 内存单位 IEC（MiB/GiB）；历史统计用采样值，明确窗口。GPU 显存属于非核心项，不在 v1.0 承诺。

### FR-03 来源追踪与证据等级（P0）

- 首次发现时立即缓存并持久化：根进程/祖先进程 PID 与 starttime、进程名、有限的脱敏命令片段、PPID、cgroup path、启动/观察时间。
- 基于进程名和链路进行候选来源分类：`OpenCode / Codex / Claude Code / Playwright / Puppeteer / Other / Unknown`。
- **不能把 Playwright 与 OpenCode 混为一谈**：`Launcher`（OpenCode）与 `Browser driver`（Playwright）是两个独立字段。
- 置信度：`Observed (直接观察到父链)`、`Inferred (基于命令/cgroup推断)`、`Unknown`；每个结论可点开查看证据。
- 对已存在于监控启动前的进程不得推断消失的祖先；不可追溯时诚实显示 Unknown。
- 历史中保留进程退出时刻和当时来源证据，不依赖事后读取 `/proc`。

### FR-04 残留与异常识别（P0）

- 状态：`Normal`、`Parent exited`、`Suspected leftover`、`High CPU`、`Stale data`；允许组合。
- `Parent exited` 是观察事实，不等于“不该存在”。
- 默认疑似残留规则：在观察到 Agent 退出后，Chrome 仍存活超过 60s；可与持续 CPU > `1.0 cores`、运行时长、用户标注白名单结合升级提醒级别。
- 允许为长任务设 `Ignore this session`、`Ignore this launcher`、`Snooze 30 min`。
- 不根据 PID 是否为 1 的子进程，直接声称原始 launcher 是 OpenCode。
- **默认仅提醒、不自动清理**。自动清理是 Post-v1 的明确 opt-in 策略，且不支持来源不确定的会话。

### FR-05 浏览器内容可视化（P1）

- 无 CDP：显示进程指标、命令、父链、分类、退出/清理操作；明确标注 `Process monitoring only`。
- 有 CDP：仅探测经本机进程启动参数确认的 `127.0.0.1` 端口；展示标签页数量、标题、URL（界面按需展开，默认遮蔽查询参数）。
- 截图为**用户点选后触发**；不做后台定时截图；可选择是否保留，默认不落盘。
- 明确区分 `CDP available`、`CDP pipe only`、`No debug endpoint`、`Endpoint unreachable`。
- 不尝试从外部抢占 `--remote-debugging-pipe`：该模式通常没有可供独立进程直接使用的 TCP 接口。
- 未来连续预览使用 CDP screencast 时需经过明确授权，并设置速率/带宽/焦点限制；不作为 v1.0 交付阻塞项。
- CDP 控制能力有隐私与安全风险：v1.0 默认只读，不提供任意 JavaScript 执行、网址导航或接管输入。

### FR-06 安全进程操作（P0）

- 操作：`Stop gracefully (SIGTERM)`；若确认进程仍在，允许 `Force kill (SIGKILL)`；新增 `Copy diagnostic report`。
- 所有破坏性操作显示确认对话框，至少包含会话来源、根 PID、进程数、影响范围、不可撤销提示。
- 后端执行前重新验证 PID + starttime + 同 UID + 浏览器特征；不按仅 PID 的过期请求终止新进程。
- 默认只结束**当前可验证的浏览器子树**；不 kill 父级 OpenCode、Node、IDE 或同一用户其他 Chrome。
- SIGTERM 后观察进程退出结果；未退出时提示用户手动确认强制结束；不暗中自动升级 SIGKILL。
- 每次操作记录：时间、会话标识、操作类型、发出信号数量、成功/失败结果、权限错误；即使服务重启仍可回溯。
- 批量清理 v1.0 仅支持明确多选且二次确认；禁用“Kill all Chrome”一键无差别执行。

### FR-07 通知、静默与告警（P1）

- 默认桌面通知：新浏览器启动、疑似残留、持续高 CPU、停止结果失败。
- CPU 告警默认 `>1.0 cores` 持续 `30s`（可配）；不要对 CPU 瞬时采样或单核 105% 直接警报。
- 告警去重：同会话同级别 `10min` 内不重复弹；状态恢复再恶化允许重新触发。
- 支持全局静音、专注模式、单会话静音；通知点击打开对应会话详情。
- 通知不可用时，Dashboard 告警中心仍完整工作。
- 重要：桌面通知不得显示敏感 URL 或完整命令行。

### FR-08 历史、搜索、导出（P1）

- SQLite WAL：保存会话生命周期、来源证据、事件、操作审计、聚合指标。
- 保留策略默认：原始每秒指标 `24h`；一分钟粒度汇总 `7d`；事件/会话元数据 `30d`；用户可以清空历史。
- 搜索/筛选：会话状态、启动者、浏览器类型、CPU 高低、持续时间、关键字、时间范围。
- 可导出 `JSON/CSV` 事件与指标摘要，**默认脱敏** URL 参数、路径、令牌、Cookie/Authorization 相关内容。
- 默认不保存截图，不收集系统其他进程的完整命令行，只保存用于归因的必要字段。

### FR-09 安装与运行体验（P0）

- 提供 Linux x86_64 独立运行包；优先单后端可执行文件 + 随包静态前端，尽量无运行时依赖。
- `headless-watch start/status/stop/open` CLI；支持 systemd 用户服务 `enable --now`、卸载脚本。
- 默认监听 `127.0.0.1:8765`；启动后提示 URL。不要默认监听 0.0.0.0。
- 有图形桌面时可选托盘图标，显示会话数与当前资源异常；Wayland/X11 都不依赖读取窗口画面。
- 重启服务时不遗失历史，端口被占用要有易懂错误和安全退出；不得自动杀掉占用同端口的其他程序。
- 监控自身也必须有稳定退出、日志大小限制与更新后状态恢复。

---

## 5. UI / UX 设计规范 —— “Invisible Browser Control Center”

### 5.1 视觉定位

**风格关键词**：Mission Control、Developer Observatory、精密仪表、低对比背景、强信息层级、动态光效。不是炫技的赛博霓虹，也不是传统 Admin模板。

产品体验：第一次打开就能看懂核心数字；细节能钻取到进程树；高级用户获得证据，不需要从十张图里猜原委。

| Token | 建议值 | 用途 |
|---|---|---|
| `--bg` | `#080C13` | 主背景，深墨蓝黑 |
| `--surface` | `#101826` | 面板层 |
| `--surface-raised` | `#162236` | 悬停、抽屉、弹窗 |
| `--border` | `#26354A` | 微弱但可分辨边界 |
| `--text-primary` | `#EAF1FB` | 主文字 |
| `--text-secondary` | `#9FB1CA` | 注释与次要值 |
| `--mint` | `#4BE4BA` | 正常活动、实时连接 |
| `--cyan` | `#70C9FF` | 数据高亮、交互焦点 |
| `--amber` | `#FFC06D` | 疑似异常 |
| `--red` | `#FF7182` | 高风险 / 破坏操作 |

- 布局：8pt 间距系统；圆角 12–16px；圆角不做过度软萌；1px 边框及弱阴影，慎用大面积毛玻璃。
- 文字：现代无衬线字体（系统优先、开源字体可选）；数字采用等宽或 tabular numerals；标题清晰，拒绝过多全大写标签。
- 动效：仅实时脉冲、小范围图表流动、行状态变化；遵循 `prefers-reduced-motion`。
- 必须支持 1366×768、1440p 与 4K 缩放；窄屏改为单栏。使用 WCAG 合适的文本对比，不能靠红绿颜色独立传达状态。

### 5.2 信息架构

**左侧导航**：Overview / Live Sessions / History / Alerts / Settings。左下角显示 Monitor Health（连接状态、采样延时、版本）。

**主工作区：**

```text
┌────────────────────────────────────────────────────────────────────────────────────────┐
│ ◉ HEADLESS WATCH                 LIVE  ●     8 sessions     Last scan 0.8s    ⚙       │
├──────────────┬─────────────────────────────────────────────────────────────────────────┤
│ Overview     │ Headless Browser Observatory                                            │
│ Live         │ ┌────────────┐ ┌────────────────┐ ┌─────────────┐ ┌─────────────────┐  │
│ History      │ │ ACTIVE  8  │ │ CPU 3.45 cores │ │ MACHINE 21% │ │ SUSPECTED   2   │  │
│ Alerts   2   │ └────────────┘ └────────────────┘ └─────────────┘ └─────────────────┘  │
│ Settings     │ ┌──────────────────────────────────────────────────────────────────────┐ │
│              │ │ CPU / MEMORY · live  (core-seconds, clearly labeled)               │ │
│              │ │   ╱╲__╱╲_____╱╲___       Window: 5m / 30m / 1h                     │ │
│              │ └──────────────────────────────────────────────────────────────────────┘ │
│              │ Active sessions                          [Source ▾] [Risk ▾] [Search]│
│              │ ┌─────────────────────────────────┐ ┌─────────────────────────────────┐ │
│              │ │ ◉ Chrome Headless    OPEN CODE │ │ ◉ chrome-headless-shell UNKNOWN│ │
│              │ │ 1.05 cores  | 6.6% machine     │ │ 0.03 cores  | 0.2% machine      │ │
│              │ │ PSS 845 MiB | 7 processes      │ │ RSS ~320 MiB | 4 processes      │ │
│              │ │  ▁▁▃▇▆▄  Live · 14m           │ │  ▁▁▁▁▁▁  Parent exited          │ │
│              │ │ [Inspect]              [Stop]  │ │ [Inspect]              [Stop]   │ │
│              │ └─────────────────────────────────┘ └─────────────────────────────────┘ │
│              │ Activity ──────────────────────────────────────────  View full history │
│              │ 15:43  Chrome started · OpenCode     15:49  Source exited · PID 1234 │
└──────────────┴─────────────────────────────────────────────────────────────────────────┘
```

### 5.3 四个重点页面

**A. Overview（默认页）**

- 顶部四项：`Active sessions`、`CPU cores`、`System CPU share`、`Suspected leftovers`；每项有精简口径说明。
- 中部一张 CPU / Memory 合并时间轴，但 Y 轴和单位各自明确，不使用误导双轴；可切换指标。
- 下方高密度会话卡片：图标、来源、状态、CPU、内存、持续时间、迷你曲线、详情入口。
- 底部事件时间线，支持点击定位会话。
- 没有会话时提供优雅空状态：`No headless browsers detected · Monitoring is active`，而不是报错。

**B. Live Sessions（核心操作页）**

- 可切换 `Cards` 与 `Compact Table`，后者适合数十个进程。
- 排序：CPU（默认）、Memory、Runtime、Recent、Risk；即时搜索和按 Agent 分组。
- 行/卡点击后从右侧打开 400–520px Inspector，不离开 Live 列表。
- Inspector 四个选项卡：`Overview`、`Process Tree`、`Browser`、`Events`。
- `Process Tree` 把 `OpenCode → Node/MCP → Chromium → Renderer` 显示为分层关系，**观察到的链路实线、推断的链路虚线**；清楚显示证据等级。
- `Browser`：CDP 可用时显示隐私遮蔽的标题、URL 和按需截图；不可用时显示明确原因，不是空白面板。
- 危险动作放在固定的 Inspector 底部；不可误触。

**C. History（事后排障页）**

- 会话生命周期时间带（Gantt-like）：每条浏览器会话对应一个时间区段。
- 对比 `Agent exit` 与 `Browser exit`，突出“Agent 退出后浏览器还运行了多久”。
- 点击历史会话，可读取历史 CPU 峰值、持续时长、原始来源证据与操作记录。
- 导出经过脱敏的诊断报告。

**D. Alerts & Settings（可靠性控制页）**

- Alerts：待处理 / 已确认 / 已静音；能够追溯为何触发，关闭弹窗不代表已解决。
- Settings：采样周期、持续 CPU 阈值、告警静音、历史保留期、CDP 开关、隐私与脱敏、安装/更新状态。
- 高危开关不得放在一排普通 toggle 中，必须解释影响范围。

### 5.4 交互细节

- 从通知点击进入对应会话，定位到 CPU 异常那一段历史。
- 会话卡状态指示的动画只作用于活动中会话；暂停、退出、数据过期状态不同。
- `Stop` 点击先询问，SIGTERM 后状态为 `Stopping`，直到采样确认退出才显示 `Stopped`。
- 键盘快捷键：`/` 搜索、`Esc` 关闭 Inspector、`?` 打开快捷键说明；其他快捷键待用户测试后决定。
- UI 区分 `Unavailable`、`Not sampled`、`0`、`Not permitted`、`Stale` 五类状态，禁止一律显示为 0。
- 页面刷新后保持筛选与时间范围；危险操作不会因页面恢复自动重放。

---

## 6. 技术架构

```text
          /proc (same UID)             Optional CDP (only existing local endpoint)
                 │                                     │
                 ▼                                     ▼
   ┌────────────────────────────────────────────────────────────────┐
   │           Headless Watch Core · Rust / Tokio                    │
   │ Process Scanner   Session Tracker   Metrics   CDP Adapter     │
   │ Origin Snapshot   Rule Evaluator    Actions   Notification     │
   └────────────┬─────────────────────────┬──────────────────────────┘
                │                         │
           SQLite WAL              Axum HTTP + SSE
                │                         │
     Session/Event History             Vue 3 SPA
                │                         │
          Export / Cleanup          Local Dashboard
```

### 6.1 后端模块边界

1. **proc_collector**：负责 `/proc` 读取、进程身份与资源采样，不包含 UI 逻辑。
2. **session_tracker**：构造浏览器会话、聚合进程、跟踪退出、计算 CPU delta 与可靠性状态。
3. **attribution**：父链与 cgroup 快照、来源分类、证据等级。
4. **cdp_adapter**：仅用于已有端口；能力探测、页面列表、用户主动截图。
5. **rules**：告警阈值、持续时间、去重、忽略规则。
6. **actions**：安全信号发送、二次验证、审计、结果确认。
7. **storage**：SQLite、迁移、保留期清理、导出。
8. **http_server**：只提供本机 API、SSE 与静态前端；安全控制在这里统一执行。
9. **frontend**：Vue 3 + TypeScript；组件拆分 `MetricCard` / `SessionCard` / `SessionTable` / `InspectorDrawer` / `ProcessTree` / `Timeline` / `AlertCenter`。

尽量用系统能力替代多余依赖：不引入任务队列、Redis、独立 Node 服务、Docker、复杂 RBAC 或第三方 telemetry SDK。

### 6.2 本地数据模型

```text
ProcessIdentity   (boot_id, pid, start_ticks)
BrowserSession    (id, root_identity, executable, launched_at, first_seen_at,
                   ended_at, discovery_mode, cgroup, launcher, driver, confidence)
ProcessSnapshot   (session_id, identity, parent_identity, role, command_redacted,
                   observed_at, exited_at)
MetricSample      (session_id, ts, cpu_cores, rss_sum, pss_sum_nullable,
                   process_count, sample_quality)
SessionEvent      (id, session_id, ts, type, severity, evidence_json_redacted)
ActionAudit       (id, session_id, ts, action, target_identities, result)
AlertRule         (id, type, threshold, duration, cooldown, enabled)
Settings          (key, value)
```

重要字段中的绝对路径、CLI 参数、URL 必须按隐私规则处理；不要把完整环境变量写进数据库。浏览器会话 ID 由程序生成，不依赖 PID 单独作为主键。

### 6.3 API 最小集合

```http
GET  /api/v1/health
GET  /api/v1/overview
GET  /api/v1/sessions?status=active&sort=cpu
GET  /api/v1/sessions/{id}
GET  /api/v1/sessions/{id}/metrics?window=30m
GET  /api/v1/events?since=...&limit=100
GET  /api/v1/stream                 # SSE，状态变化与指标推送
GET  /api/v1/sessions/{id}/tabs     # 仅已有且获准的 CDP
POST /api/v1/sessions/{id}/screenshot
POST /api/v1/sessions/{id}/terminate # 必须提供 identity + mode + CSRF token
GET  /api/v1/settings
PATCH /api/v1/settings
GET  /api/v1/export?format=json
```

- 全部接口本机 loopback 访问；变更状态的 API 要求随机 token + Origin/Host 检查。
- 后端拒绝从客户端任意传入地址用于 CDP 连接；CDP 目标必须从可信的本机浏览器进程证据导出，防止 SSRF。
- SSE 自动重连、序列号去重；首次连接返回快照，后续推送变化；客户端长时间失联后重新取快照。
- 不开放跨域写访问；如果未来允许远程访问，需**重新设计认证**，不能直接把 127.0.0.1 改为 0.0.0.0。

---

## 7. 可靠性、安全、隐私与性能

### 7.1 权限与数据安全

- 原则：**同 UID、只读观测、按需 CDP、用户确认销毁**；无 root 服务为默认。
- **不依赖屏幕截取**，因此兼容 Wayland；CDP 预览只针对已开启调试通道的浏览器。
- CDP 能力相当敏感，不能因为“只在本机”就忽略跨站请求、本地恶意页面或 Token 泄漏的风险。
- CLI 参数可能含登录 Token，进程祖先可能包含私有工作目录；采集层脱敏，展示层支持隐藏。
- 数据库权限设为用户可读写（`0600`）；不默认上传遥测，不记录浏览器密码、Cookie、POST 请求正文。
- 命令中的 `--remote-debugging-port=0` 和 DevToolsActivePort 只是**可以发现已有 CDP 的条件**，不得主动改写别人的启动参数。

### 7.2 性能目标（验收目标，而非宣称已达成）

- 默认后台扫描平均 CPU 开销 **< 1% 单逻辑核心**，在典型开发机、无 CDP 截图、数十个同 UID 进程的工况下测量。
- 后端常驻内存目标 **< 100 MiB**；前端独立计量，不合并掩盖。
- 新 Chrome 持续存活时，最迟 **2s** 内进入 Dashboard（默认 1s 扫描 + 推送缓冲）。
- 正常桌面下 100 个浏览器子进程仍保持流畅，不因 PSS 读取或 CDP 请求阻塞扫描。
- 历史总数据可设置空间上限，过期自动清理；离线/权限错误不得导致无界重试或无限日志增长。

### 7.3 错误边界

- `/proc` 瞬态消失、读取权限不足、进程重用 PID、Chrome 快速退出、CDP 拒绝连接、标签页关闭时截图、后端重启、时钟跳变、资源紧张、端口占用均需有确定的预期行为。
- Chrome 大量派生与频繁退出时不出现 CPU 极端虚高；不得误杀 PID 被复用的新进程。
- Browser monitoring 可用 ≠ CDP accessible；UI 明确分开表示。
- 健康探针独立于浏览器列表；全局扫描异常时不能显示“没有浏览器”造成误解。

---

## 8. 验收用例与发布门槛

| ID | 场景 | 验收标准 |
|---|---|---|
| AT-01 | 用户启动普通图形 Chrome | **不出现** Headless 会话 |
| AT-02 | `chromium --headless` 启动 | 默认 2s 内出现，结束后进入历史 |
| AT-03 | 1.05 逻辑核心，16 CPU | 同时显示 `105% single-core`、`1.05 cores`、约 `6.6% machine` |
| AT-04 | 新 renderer 中途加入已存在会话 | 聚合 CPU 不因旧的累计 ticks 突然跳高 |
| AT-05 | 多个 Chrome 共享动态链接库页面 | `ΣRSS` 明确为估算；若有 PSS 展示准确度和时间 |
| AT-06 | OpenCode → MCP → Chrome，在监控运行期间父进程退出 | 历史父链依然保留；弹出 Parent exited / 疑似残留而非断言“恶意进程” |
| AT-07 | 监控启动前已有 Chrome，来源消失 | `Unknown origin`，不得编造来源 |
| AT-08 | CDP port 存在 | 可读取列表；按需截图，不后台录屏 |
| AT-09 | 仅 `--remote-debugging-pipe` | 仍可看资源指标；提示无外部 CDP 附加能力 |
| AT-10 | Stop 和 Force kill | 明确二次确认、验证 PID starttime、完整审计，不影响正常 Chrome 和 OpenCode |
| AT-11 | PID 被系统复用 | 旧卡的结束命令必须拒绝操作新进程 |
| AT-12 | 后端重启或 Web 端刷新 | 历史仍存在；UI 重连正常，不重复执行终止操作 |
| AT-13 | 攻击者从恶意网页尝试访问 localhost API | 非授权的 POST/截图请求失败；不能连接任意 CDP URL |
| AT-14 | 多会话与持续高 CPU | 告警依据持续阈值，静音与去重工作；不把 105% 自动当整机 105% |
| AT-15 | Wayland 会话 | 正常监测、查看 Web Dashboard、触发桌面通知（若通知服务可用） |
| AT-16 | 无头浏览器突然 crash，进程表异步变化 | UI 正常收敛、进程状态准确、后台线程不退出 |
| AT-17 | 非预期后台进程失败或端口冲突 | 错误可见，服务可安全停止/重启，不存在后台孤儿监控进程 |
| AT-18 | 100 个浏览器相关子进程的压力场景 | 满足定义好的资源开销、UI 响应与数据库大小目标 |

发布前使用**真实 OpenCode + Playwright MCP**至少跑一轮完整生命周期验证，覆盖：正常退出、强制中断、Agent 退出而 Chrome 仍在、CDP 可用和不可用两类浏览器。所有危险动作都需自动化回归测试，不只人工点 UI。

---

## 9. 分阶段实施与交付物

### Milestone A — 修正核心事实（v0.2）

- 基于当前 MVP 先校正 CPU、PSS/内存口径、逐 PID delta、PID identity 与观察质量。
- 保留现有浏览器自动发现与 screenshot 功能；加回归测试。
- **验收门**：AT-01/02/03/04/05/09/11。完成之前不迁移框架，以免掩盖数据缺陷。

### Milestone B — 正式后端与 UI（v0.5）

- Rust 采集服务替代单文件 Python；Axum、SQLite WAL；Vue3 深色控制台。
- 来源快照、指标历史、会话详情、Inspector、告警、通知与搜索。
- 保留当前启动 URL 与 systemd 用户服务体验；迁移脚本完成后不要求双后端同时运行。
- **验收门**：AT-06/07/08/12/14/15；首次使用流程可完成。

### Milestone C — 可公开发布（v1.0）

- 安全清理、批量确认、来源风险证据、脱敏、端到端真实测试、打包、卸载、完善 README、许可证与版本发布。
- 如托盘开发明显增加复杂度，可只做极简系统状态入口，不阻塞浏览器监控的核心质量。
- **验收门**：全部 AT 用例通过，特别是 AT-10/11/13/16/17/18。

### Post-v1 — 受管理会话模式（单独立项）

可提供 `headless-watch run -- opencode`，使用 systemd transient unit/cgroup 将 Agent 与子进程纳入单独生命周期，获得更强的“退出即清理”控制；必须告知对 Agent 子进程的影响，和普通监控模式严格分离。严禁以“自动接管任意 Chrome”作为实现目标。高级 eBPF 追踪属于可选的特权增强模块，不阻塞主产品。

---

## 10. 给 Coding Agent 的实施约束

1. 阅读当前三份文件：`headless_watch.py`、`README.md`、`install.sh`；先确认实际行为，不删除现有可用功能。
2. 只按本 PRD 的 v0.2 → v0.5 → v1.0 迭代；先把数据正确性与安全用例测试通过，再重做 UI。
3. 不引入不必要的服务、平台、框架或调度器；**复用 Linux `/proc`、systemd、CDP、SQLite 能力**。
4. 每项需求给出“实现 → 可复现实测 → 回归测试”三件套；不以类型检查通过代替真实浏览器测试。
5. 不默默改变用户的 Chrome、CDP 或 OpenCode 参数，不访问外网，不使用用户真实个人浏览器资料进行测试。
6. UI 必须对真实 API 工作；不交付静态假 Dashboard 或无法被后端驱动的数字。
7. 不做无意义的 PR 分拆，不允许为了美化 UI 破坏监控准确性、退出清理与权限保护。
8. 每个阶段提交必要的启动方式、环境依赖、回滚手段及未解决限制；失败时维持旧版可工作。

---

## 11. 参考标准与技术资料

- Linux `/proc/PID/stat` 的 `utime/stime/starttime` 字段与单位：https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
- Linux `/proc`、`smaps_rollup` 与 PSS 语义：https://docs.kernel.org/filesystems/proc.html
- Chrome DevTools Protocol / `DevToolsActivePort` / websocket 目标：https://chromedevtools.github.io/devtools-protocol/
- systemd 资源控制 / `CPUQuota=`：https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html

以上资料用于明确能力边界。PRD 的性能目标、设计规格及时间窗口是产品建议，不表示相关工具已经实现或验证。

---

**验收定义（Definition of Done）**：用户在 Ubuntu 登录后不需要操作 OpenCode 的配置；当一个新的无头浏览器出现，能够准确地看到它、追溯能证明的来源、理解它用了多少机器资源；Agent 异常退出后历史不丢，用户能在受保护的界面中安全结束浏览器；不存在“看起来很酷但显示错误数字”的情况。
