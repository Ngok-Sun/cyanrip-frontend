# cyanrip-frontend 开发计划

> 状态：M1（core + CLI）、M2（GUI 五页）、M3（元数据与标签）已完成并通过真碟验收；
> M3 之后的交互/封面/多碟/多实例/HDCD 等增补也已完成（见 §15.3）。
> 剩余：队列与自动换碟为主（§15.4）
> 实施进度见 §15，自测开关见 §16，变更日志见 §17
> 编写日期：2026-11-06
> 目标平台：Linux（GNOME / Wayland 优先），Rust + GTK4 + libadwaita
> 关联技能：`cyanrip-cd-ripping`（抓轨流程、驱动坑、元数据链路）

---

## 1. 项目概述

### 1.1 目标

为 `cyanrip` 提供一个原生 GTK4 图形前端，让"放碟 → 选轨 → 抓轨 → 校验 → 打标签 → 归档"整条链路在一个窗口内完成，且**校验结论与命令行完全一致**（AccurateRip / EAC CRC32 可追溯）。

### 1.2 设计原则

1. **不重新实现 CD 读取**：所有读取、纠错、校验交给 `cyanrip` 子进程，GUI 只做编排、解析与呈现。
2. **可追溯**：每张碟产出的 EAC 风格 log、等价命令行、AR 命中的 confidence 都可在界面内查看与导出。
3. **不阻塞 UI**：GTK 主线程只渲染；所有 I/O（子进程、HTTP）在 tokio runtime，事件回投主循环。
4. **离线可用**：元数据查询失败不阻塞抓轨；无网络时按 `Unknown CD <日期>` 落盘，事后再补标签。
5. **参数透明**：界面随时显示等价的 `cyanrip` 命令行，避免"界面一套、实际一套"。

### 1.3 非目标（本阶段不做）

- Windows / macOS 支持（GTK4 分发成本高，另行评估）
- 内嵌音频播放（GStreamer 已在系统内，留作后续）
- 自行实现 CD-DA 读取、C2 纠错、AccurateRip 计算
- 音乐库管理（不替代 beets，只负责单张/整箱碟的抓取与标签写入）
- CD-TEXT 编辑、封面 OCR、歌词抓取
- 抓轨之外的自动化（上传 NAS、触发 beets 导入、远端归档等）——留作后续扩展
- 多语言界面（本阶段只出中文，但所有用户可见字符串必须走 i18n 通道，见 §7.6）

---

## 2. 技术栈

### 2.1 系统依赖

| 组件 | 版本要求 | 本机现状 |
|---|---|---|
| Rust | stable ≥ 1.75（rustup 安装） | **未安装** |
| GTK4 开发头文件 | ≥ 4.10 | 运行库 4.14.5，`libgtk-4-dev` **未安装** |
| libadwaita 开发头文件 | ≥ 1.4 | 运行库 1.5，`libadwaita-1-dev` **未安装** |
| cyanrip | ≥ 0.9.4（含 MEDIA_CHECK 限流） | `/usr/local/bin/cyanrip` 0.9.4-rc3 ✅ |
| cdrdao | 任意 | 1.2.4 ✅（读 TOC / ISRC） |
| fpcalc (chromaprint) | 1.5+ | 1.5.1 ✅ |
| metaflac / flac | 1.4+ | 1.4.3 ✅（**标签写入与 PICTURE 内嵌都用 metaflac**） |
| canberra-gtk-play | 任意（提示音） | ✅ 已安装；回退 `pw-play` |

安装命令：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
sudo apt install -y libgtk-4-dev libadwaita-1-dev build-essential pkg-config
```

> 注意：本机 shell 的 `python3` 指向 Hermes venv，无 `gi`；GUI 不用 Python，仅记录以免混淆。

### 2.2 Rust crates

- GUI：`gtk4`、`libadwaita`（`gtk4` crate 需与系统 GTK 版本匹配，0.9.x 支持 4.10+）
- 异步：`tokio`（process / io-util / sync / time / rt-multi-thread）、`async-channel`
- 序列化：`serde`、`serde_json`、`toml`
- 网络：`ureq`（rustls，`json` 特性；无 async 运行时负担）
- Disc ID：`sha1`（自行拼串 + libdiscid 的 base64 变体）
- CUE 编码：`encoding_rs`（UTF-8 → GBK 回退）
- 标签：调用 `metaflac`（不引 lofty；已用真碟验证不会动音频流）
- 错误：`thiserror`（库层）、`anyhow`（应用层）
- 日志：`tracing`（CLI）
- 其他：`directories`、`chrono`、`regex`、`libc`（ioctl）
- 国际化：`gettext-rs`（配合 `xgettext` 提取，`po/zh_CN.po` 随包）
- 提示音不引入 Rust 依赖：调用 `canberra-gtk-play` / `pw-play`，避免为一段音效拉 gstreamer-rs 及 dev 头文件

### 2.3 不引入的依赖

- relm4：本项目页面数少，手写 `AppState + Msg` 胶水更易调试（约 200 行）
- Tauri / webview：为桌面单机工具引入前端栈得不偿失
- 任何 CD 读取库（libcdio FFI）：读取统一走 cyanrip/cdrdao 子进程

---

## 3. 工程结构

```
cyanrip-frontend/
├─ Cargo.toml                     # [workspace] members = crates/*
├─ DEVELOPMENT_PLAN.md            # 本文件
├─ README.md
├─ assets/
│  ├─ icons/                      # 应用图标、状态图标
│  └─ style.css                   # 少量自定义样式
├─ tests/
│  └─ fixtures/                   # 真实 cyanrip log、MB/Discogs JSON 快照
│     ├─ log_quartet.txt          # 久石譲 Quartet，13/13 AR conf 8
│     ├─ log_flower_cd1.txt       # 花儿乐队 CD1，15/16 AR
│     ├─ log_flower_cd2.txt       # 花儿乐队 CD2，8/8 AR
│     └─ json/                    # 录制好的 API 响应
└─ crates/
   ├─ cyanrip-core/               # lib，不含 GTK
   │  └─ src/
   │     lib.rs
   │     disc.rs                  # TOC / 碟片信息（cdrdao read-toc 解析）
   │     rip.rs                   # 启动、驱动、取消 cyanrip
   │     progress.rs              # '\r' 进度帧解析
   │     logparse.rs              # EAC log → 结构化结果
   │     event.rs                 # 事件枚举（UI 唯一输入）
   │     job.rs                   # 任务/队列状态机
   │     config.rs                # 配置读写与默认值
   │     tag.rs                   # FLAC 标签与封面写入（metaflac）+ 归档/重命名
   │     meta/
   │       mod.rs                 # 元数据源抽象 + 优先级调度
   │       musicbrainz.rs
   │       acoustid.rs
   │       discogs.rs
   │       coverart.rs
   └─ cyanrip-gui/                # bin，GTK4
      └─ src/
         main.rs                 # AdwApplication
         state.rs                # AppState + Msg
         ui/
           window.rs             # HeaderBar / ViewStack / 队列侧栏
           page_disc.rs          # 碟片 + 曲目表 + 快捷参数
           page_settings.rs      # 完整设置页（五分类）
           page_progress.rs
           page_result.rs
           page_meta.rs          # 未识别碟的手工/半自动匹配
           log_view.rs
           widgets/              # 模板预览、参数摘要条、轨道行
```

M4 增加 `crates/cyanrip-cli/`（同一 core 的无头入口，替代现有 `ripcd`）。

---

## 4. 架构

### 4.1 分层

```
┌──────────────────────────────────────────┐
│ cyanrip-gui (GTK 主线程)                  │
│   AppState ⇄ Msg ⇄ 视图                   │
└───────────────▲──────────────────────────┘
                │ Event (async_channel)
┌───────────────┴──────────────────────────┐
│ cyanrip-core (tokio)                     │
│  Job ──► RipRunner ──► cyanrip 子进程     │
│        └► MetaResolver ──► HTTP          │
│        └► LogParser / ProgressParser      │
└──────────────────────────────────────────┘
```

- UI 侧只处理 `Msg`（用户操作）与 `Event`（core 上报），不直接持有 tokio handle
- core 不依赖 GTK，可用 `cargo test` 完全离线验证

### 4.2 并发与线程模型

- GTK 主线程：`glib::MainContext`，只做 UI 更新
- 后台：`tokio::runtime::Runtime`（multi-thread，默认 2 worker 足够）
- 桥接：`async_channel::unbounded<Event>`；UI 用 `glib::MainContext::spawn_local` 消费
- 子进程：`tokio::process::Command`，stdout/stderr 各一个 reader task
- 进度节流：core 侧把 `\r` 帧归并到 ≤10 Hz 再发 `Event::Progress`，避免刷爆主循环
- 禁止：`block_on`、在主线程做 HTTP、`std::process::Command::wait`

### 4.3 取消语义

```
用户点取消 → 二次确认 → Event::CancelRequested
  → core 向子进程发 SIGTERM，2s 后 SIGKILL
  → 删除"抓取中"轨的残片（*.flac.part / 半成品）
  → 已完成轨保留，Job 置 PartialDone，结果页标黄
```

`-Z 2` 模式下已完成的校验结果不丢失；日志尾部追加 `cancelled by user at track N`。

### 4.4 状态机

```
Idle → ProbingDisc → DiscReady → Ripping → Verifying(解析log) → Done
                        │            │
                        │            └→ Cancelled / Failed
                        └→ NoMedium（无碟/换碟）→ 轮询等待
```

---

## 5. 数据模型

### 5.1 事件（core → UI）

```rust
enum Event {
    DiscProbed(DiscInfo),
    DiscRemoved,
    TrackStarted { index: u32, title: String },
    Progress { track: u32, track_pct: f32, overall_pct: f32, speed_x: f32, eta: Duration },
    LogLine(String),                 // 原始行，供日志面板
    TrackDone { index: u32, crc: Option<u32>, ar: ArStatus },
    RipFinished(RipReport),
    RipFailed { stage: Stage, message: String },
    MetaResolved(MetaCandidate),
    MetaFailed(String),
}
```

### 5.2 关键结构

- `DiscInfo { device, model, firmware, disc_id, cddb_id, tracks: Vec<TrackInfo>, total_duration, ar_in_db: bool, isrcs: Vec<Option<String>> }`
- `TrackInfo { number, start_lsn, duration, title: Option<String>, selected: bool }`
- `ArStatus { v1: Option<u32>, v2: Option<u32>, confidence: Option<u8>, state: InDb | NotFound | Mismatch }`
- `RipReport { offset: i32, c2_supported: bool, paranoia_level: u8, tracks: Vec<TrackResult>, errors: u32, started_at, finished_at, log_path }`
- `Settings { device, offset, speed, paranoia, retries, verify_passes, overread, use_c2, output…, meta…, behavior… }`

### 5.3 配置文件

`~/.config/cyanrip-frontend/config.toml`

```toml
[device]
path = "/dev/sr0"
# 按设备型号记忆偏移，换光驱自动匹配
offsets = { "PIONEER DVD-RW DVR-106D" = 48 }

[rip]
paranoia = 3
retries = 10
verify_passes = 2      # -Z
overread = false
use_c2 = false         # 本机驱动不支持 C2
formats = ["flac"]     # 可多选并行编码
image_mode = false

[output]
root = "~/Music"
dir_template = "{artist}/{album}"
file_template = "{track:02} {title}"
log_template = "{album}"
write_cue = true
embed_cover = true
replaygain = true

[meta]
priority = ["musicbrainz", "acoustid", "discogs"]
language = "zh"
cover_max_px = 1200

[behavior]
auto_tag_after_rip = true
eject_when_done = true
notify = true                 # 桌面通知
sound = true                  # 抓完播放提示音
sound_theme = "complete"      # freedesktop 主题音（canberra-gtk-play -i）
sound_file = ""               # 非空则播放自定义音频（canberra -f，回退 pw-play）
shutdown_when_done = false    # 抓完关机，默认关，开启需二次确认
on_error = "continue"         # continue | abort
unknown_disc = "pause"        # pause（等待人工指定）| rip_placeholder（按 Unknown CD 落盘）

[ui]
language = "zh"               # 本阶段只打包 zh，其余语言留接口
```

密钥单独存放：`~/.config/cyanrip-frontend/secrets.toml`（0600），只存 Discogs token。

---

## 6. GUI 参数 → cyanrip 参数映射

| 界面项 | cyanrip 参数 | 备注 |
|---|---|---|
| 设备 | `-d /dev/sr0` | 自动枚举 `/dev/sr*` |
| 读偏移 | `-s 48` | 必须显式；按设备记忆 |
| 速度 | `-S N` | 本机报 "Device does not support changing speeds!"，UI 需容忍 |
| Paranoia | `-P 0..3` | 默认 3 |
| 帧重试 | `-r N` | 默认 10 |
| 校验次数 | `-Z N` | 默认 2（≈3 遍读取） |
| Overread | `-O` | 提示风险 |
| C2 | （cyanrip 自动） | 本机不支持，设置页只做说明与告警 |
| 格式 | `-o flac,mp3` | 多格式并行编码 |
| 目录模板 | `-D` | 自定义变量映射 |
| 文件模板 | `-F` | |
| 日志名 | `-L` | |
| CUE 名 | `-M` | |
| 选轨 | `-l 1,3-5` | |
| 专辑/曲目元数据 | `-a album=…:album_artist=…`、`-t n=title=…` | 未命中 MB 时使用 |
| MB 发行版 | `-R <index|id>` | |
| 跳过元数据 | `-N` | 未识别碟 |
| 采集/HDCD 等 | `-H` `-E` `-W` `-K` `-X` `-p` | 高级分页 |

参数摘要条实时拼出等价命令行，可复制。

---

## 7. UI 设计

### 7.1 主窗口

`AdwApplicationWindow`（1080×700），内容由 `AdwToolbarView` + `AdwHeaderBar` + `GtkStack` 组成
（AdwApplicationWindow 不支持 `set_titlebar`，必须用 ToolbarView）。顶部 StackSwitcher 切页；
「开始抓轨」不在标题栏，位于碟片信息区（见下）

```
┌────────────────────────────────────────────────────────────────────┐
│ 碟片 │ 进度 │ 结果 │ 设置                                  [⚙]        │
├────────────────────────────────────────────────────────────────────┤
│ 设备 [/dev/sr0 · PIONEER DVD-RW DVR-106D ▾] [刷新] [⏏]              │
│ 偏移 [48] [测定偏移]                                                │
│ ┌────────┐  PIONEER DVD-RW DVR-106D 1.00 · 13 轨 · 41:22            │
│ │        │  Yo‐Yo Ma — The Best of Yo‐Yo Ma（2004 · MusicBrainz）   │
│ │ 封面   │  [候选专辑 ▾] [导入曲目信息] [导出 CUE + 封面]            │
│ │180×180 │  ☐ 这张碟是 HDCD（20-bit 解码）                          │
│ │[取得封面]│           [      开始抓轨      ]                        │
│ └────────┘                                                          │
│ ☑ 全选                                                              │
│ ┌────────────────────────────────────────────────────────────────┐ │
│ │ ☑  01  Suite No. 1 in G major, BWV 1007: I. Prelude     6:12   │ │
│ └────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────┘
```

- 设备为 `GtkDropDown`，项来自 `/dev/sr*`，标签带 sysfs 读到的厂商+型号；切换即自动探测
- 「刷新」= 重扫 `/dev/sr*` + 读取当前碟片 TOC + 触发元数据识别，一个按钮做完整刷新
  （原「重新扫描」和「探测」两个按钮已合并；托盘为空时只更新状态，不弹错误）
- 「弹出」是图标按钮（`media-eject-symbolic`，带 tooltip 与无障碍标签），调 `eject`（见 §14 桥接坑）；
  **不能用 `Button::from_icon_name`**——GTK 会加 `image-button` 类，libadwaita 把它画成无边框、
  只在悬停时变灰，和旁边的文字按钮不一致。改成 `Button::new()` + `Image` 子控件 + `text-button` 类。
- 「测定偏移」调 `cyanrip -f`，结果按光驱型号记忆
- 「全选」为三态复选框：逐轨取消时自动变半选
- 曲目表用 `GtkListBox` + `AdwActionRow`（13–24 轨规模足够，未用 ColumnView）
- **封面预览**（碟片页左侧）：180px 方形，识别到候选后自动取图。取图链：
  候选自带的 URL（MB → Cover Art Archive）→ Discogs（条码/目录号/关键字）→
  **Google 自定义搜索（图片）**（设置里填 API Key + CX，缺一不启用）。
  失败会记下该候选，自动流程不再重试，点「取得封面」可强制重来；
  成功的结果按「来源:id」缓存，写标签时复用，不重复下载
- **导出 CUE + 封面**：把当前候选写成单镜像风格 CUE（PERFORMER/TITLE/REM DATE/DISCNUMBER/
  TOTALDISCS/CATALOG + 逐轨 INDEX，时间按 TOC 起算，UTF-8 BOM），同时把封面图复制成
  `<专辑>.jpg`；未启用任何刮削器时只导出 CUE
- **HDCD 复选框**（开始抓轨上方）：「这张碟是 HDCD（20-bit 解码）」。cyanrip 不自动检测，
  由用户判断；勾选后加 `-H`，输出 24-bit FLAC（体积约 +50%），日志里会打印
  `HDCD detected: yes, peak_extend…`；换碟/弹出时自动复位
- **开始抓轨**：不再放标题栏，改为**专辑候选下拉框正下方**居中的大按钮
  （320×64，1.35em 粗体，`suggested-action` 强调色，圆角 14px）
- **元数据条**（曲目表上方）：识别状态 + 候选下拉 +「导入曲目信息」（选 CUE 表，用于数据库
  里查不到的碟；CUE 作为最高优先级候选，网络结果不会把它顶掉）（重试走「刷新」）。探测成功即自动识别
  （只用 TOC + MCN，不需要音频），曲目表直接显示真实曲名；切换候选会同步更新标题
  和传给 cyanrip 的 `-a`/`-t`
- 抓轨前的识别未完成就点「开始」时，等识别结束（≤15s）再启动，避免丢掉元数据
- 同一张碟被介质轮询重复探测时保留勾选状态，只有换碟才重置成全选
- **坑**：`AdwActionRow` 的 `use-markup` 默认是 **true**，曲名里的 `&`、`<`、`>` 会被当
  Pango 标记解析；解析失败（如 `… tenor & orchestra`）时整行标题渲染为空白。
  所有动态标题的 ActionRow 必须显式 `.use_markup(false)`
- **介质轮询**：每 1.5 秒查 `CDROM_DRIVE_STATUS`，换碟/收回托盘后自动重新探测并刷新列表；抓轨期间暂停轮询

### 7.2 设置页（五分类，独立页面而非弹窗）

左侧分类列表，右侧 `AdwPreferencesPage` 滚动表单：

1. **设备与驱动**：设备、偏移（+自动检测）、速度、Paranoia、重试、`-Z`、overread、C2 说明
2. **输出**：根目录（**目录选择器**，不手打路径）、四个模板（含实时预览）、格式多选、image 模式、CUE/log/封面/ReplayGain/HDCD/de-emphasis/数据轨
3. **元数据**：源优先级（可拖动）、**Discogs access token（掩码输入，存 `secrets.toml` 0600，
   不写进 config.toml；首次会从旧的 `~/.config/discogs/token` 回填）**、**Google API Key + CX
   （封面兜底，同样存 secrets.toml）**、**封面刮削器多选（MusicBrainz / Discogs / Google，
   全不勾 = 不刮封面，也不往 FLAC 里写图）**、语言、封面来源与尺寸、标签字段映射
4. **行为**：自动打标签、抓完弹托盘、桌面通知、**抓完播放提示音（开关 + 事件下拉 + 试听）**、抓完关机（默认关，开启需二次确认）、出错策略、未识别碟处理
5. **高级/诊断**：cyanrip 可执行路径与版本探测（区分 `/usr/local/bin` 与 `/usr/bin`）、额外参数透传、日志目录、调试级别、导出诊断包、**界面语言（当前仅中文）**
6. **应用**：设置状态行 + **保存**按钮（无改动时置灰）、**恢复默认**（只改内存，仍需保存）

交互规则（实现后修正）：

- **显式保存**：改动只标记为"未保存"（状态行 + 保存按钮可用），点保存才原子写 `config.toml`；探测/换碟不会顺手把未保存改动写盘
- **能选就不打字**：路径类用 `GtkFileDialog` 选目录；提示音从 `/usr/share/sounds/freedesktop/stereo` 里存在的事件名生成下拉（36 个），并带「试听」按钮，避免手打名字写错导致没声音
- 抓轨中修改会生效于下一次的项，显示 `下一张生效` 徽标
- 每项右侧 `ⓘ` 说明代价（如 `-Z 2` 时间翻倍）

### 7.3 进度页

```
┌────────────────────────────────────────────────────────────┐
│ 轨 5                                                        │
│ 本轨   [██████████░░░░░░░░] 47.3%                           │
│ 总进度 [██████░░░░░░░░░░░░] 31.0%                           │
│ Ripping track 5, progress - 47.32%, ETA - 2m  · 2.3x · 已用 12:40 │
│ ── 日志 ────────────────────────────────────────────────── │
│  Track 1 ripped and encoded successfully!                   │
│  …                                                          │
│                                            [取消抓轨]       │
└────────────────────────────────────────────────────────────┘
```

- **进度行与终端一致**：单行等宽文本原地更新（`Ripping track N, progress - xx.xx%, ETA - Xm`），后面附倍速与已用时间
- 进度帧只走 `Progress` 事件，**不写入日志面板**；日志只保留真实行（Track 完成、Summary、警告）
- 双进度条；总进度按"每轨 N+1 遍"折算，`-Z` 重读时不会倒退
- 倍速用 ≥1s 滚动差分，ETA 在总进度 >3% 后改用"已用/完成比例"自算
- 日志面板等宽、自动滚底，超过 4000 行时截断重填

### 7.4 结果页

顶部一句话结论，下面是概览键值区与逐轨明细表：

```
13/13 轨精确命中 · 全部精确命中，与 AccurateRip 数据库一致

设备      PIONEER DVD-RW DVR-106D 1.00
碟片      13 轨 · 41:22
读偏移    +48 samples
C2 支持   驱动不支持，依赖多遍比对
Paranoia  max
AccurateRip 数据库中有这张碟 · 001333f2-00bfe5ad-9909b20d
精确命中  13/13
抓轨错误  0
─────────────────────────────────────────────
曲目明细
轨   时长    EAC CRC32   AR v1      AR v2      conf  状态
01   3:18    01834C83    6F096812   17AF569B   8     精确命中
16   3:14    364B3C50    E4F505C0   2CAF18F0   —     未入库
```

- 状态列带颜色：精确命中（绿）/ 未入库（黄）/ 不一致（红）/ 无数据（灰）
- CRC、AR、时长用等宽字体；所有值可选中复制
- 操作：打开输出目录、复制日志

### 7.5 元数据页（未识别碟）

- MB 命中：直接展示发行版列表，选版本 → 预览曲目映射 → 应用
- 未命中：AcoustID 候选（带 score/时长）+ 条码输入 + Discogs 搜索（结果含国家/年份/厂牌/格式）
- 选定后展示"碟轨 → Discogs 轨"的对应表，允许手工调整，再写标签

### 7.6 国际化（i18n）与完成提示

**i18n 架构（中文先行，接口留全）**

- 所有用户可见字符串走 `gettext-rs`（gettext 与 GTK 生态一致），源码里不出现裸中文字面量；日志与 cyanrip 原始输出不翻译
- 随包只带 `po/zh_CN.po`；`LANG` 或设置项决定语言，缺翻译时回退 msgid（英文原文），不出现空白界面
- 数字、时长、日期统一走格式化函数（`chrono` + 本地化），禁止在 UI 代码里手工拼 `{}分{}秒`
- 先定一份领域词表（抓轨 / 校验 / 读偏移 / 压片 / 命中），保证后续翻译一致
- 新增语言 = 加一个 `po/<lang>.po`，不改代码；M4 用 `en` 目录验证机制本身
- 设置页语言项只列已打包语言，当前仅"中文"

**完成提示音**

- 成功（含部分成功）时播放：`canberra-gtk-play -i complete`（freedesktop 主题音，本机已安装）
- 自定义音效：`canberra-gtk-play -f <path>`；canberra 不可用时回退 `pw-play <path>`
- 全部失败则静默继续，绝不阻塞结果页或弹出错误
- 提示音、桌面通知、自动弹出托盘三者独立开关；关机单独开关且默认关闭

---

### 7.7 图标与桌面项

- 应用 id：`io.github.cyanrip.Frontend`
- 图标：`assets/io.github.cyanrip.Frontend.svg`（CD + 抓轨箭头），装到
  `~/.local/share/icons/hicolor/scalable/apps/`，名字与 app id 一致
- 桌面项文件名必须等于 app id（`io.github.cyanrip.Frontend.desktop`），
  否则 GNOME 在 Wayland 下找不到窗口对应的入口，任务栏会显示通用图标
- 代码里再显式 `set_icon_name` / `set_default_icon_name`，兼顾 X11 与无桌面项场景

### 7.8 多实例（两台光驱同时抓轨）

默认是单实例应用（GApplication）：再启动一个进程只会把已有窗口提到前台。
要并行抓两张碟，需要显式开第二个实例并给它独立配置：

- `~/.local/bin/cyanrip-frontend-2`：内部设 `CYANRIP_GUI_STANDALONE=1`（允许并存）
  与 `CYANRIP_GUI_CONFIG=~/.config/cyanrip-frontend/second.toml`（独立配置）
- 两个实例各自在下拉里选自己的光驱；设备、偏移、输出根目录、模板互不覆盖
- 本次抓轨产物目录用「开始前快照 + 结束时差集」定位（`pick_rip_dir`），不再靠全局
  mtime 猜，否则并发时可能把对方还没写完的目录当成自己的去打标签
- 每个实例在 `$XDG_RUNTIME_DIR/cyanrip-frontend/<pid>.lock` 登记；「抓完关机」若发现
  还有其它实例存活就跳过关机并提示，避免先抓完的那台把机器关掉
- 仍然会冲突的情况：两个实例写**同一个输出根目录且专辑同名**（重名会失败）；
  MusicBrainz / Discogs 的限速是按应用计的，两边同时识别可能各自退避重试

## 8. 元数据与标签

### 8.1 识别链路

```
碟片 TOC + MCN（探测时即可得，不需要音频）
  ├─ 0. 手工 CUE 表（最高优先，数据库查不到的碟）
  ├─ 1. MusicBrainz discid lookup          → 按「本碟音轨数」挑 medium（套装）
  ├─ 2. Discogs：MCN/条码 → release         → 按 position 前缀分碟
  ├─ 3. AcoustID 指纹（fpcalc，需 key）     → 再用 Discogs 关键字兜底
  └─ 4. 都失败：占位命名，界面提示导入 CUE
```

实现要点：

- **Disc ID 自行计算**（SHA-1 + libdiscid 的 base64 变体），单测对照 cyanrip 日志里的真值
- **套装**：MB 的每个 medium、Discogs 的 `1-03` position 各自成候选，带 `disc_number/disc_total`；
  抓轨传 `-a disc=N:totaldiscs=M` + `-D "{album} CD{disc} [{format}]"`，标签写
  `DISCNUMBER/DISCTOTAL`，文件名带 `CD1-`/`CD2-`
- **打分**：按轨数与逐轨时长（±4s）；Discogs 不填时长时按轨数给 60 分
- **CUE 导入**：UTF-8（含 BOM）/GBK 自动解码；CUE 候选优先级最高，网络结果不会顶掉它
- **识别时机**：探测成功即开始（后台线程），不阻塞界面；识别未完成就点抓轨会等它（≤15s）
- 实测经验：
  - ISRC 不是 Discogs 的检索字段；只能用于 MB 交叉校验
  - Discogs 时长常四舍五入，容差取 ±4s
  - 中文碟常无 ISRC/CD-TEXT，CUE 手工导入是最可靠的兜底

### 8.2 封面

```
候选自带图（MB → Cover Art Archive / Discogs images）
  → Discogs 兜底（条码 → 目录号 → 艺人+专辑）
  → Google 自定义搜索（需 API Key + CX）
```

- 设置里三个刮削器可多选；**全不勾 = 完全不下图**，也不往 FLAC 里写图
- 结果按「来源:id」缓存，写标签时复用；失败会记住该候选，自动流程不重试（点按钮可重来）
- 同一候选的并发请求会去重（Google 免费额度 100 次/天）
- 写标签时：内嵌 PICTURE，并在专辑目录额外存 `cover.jpg` / `cover.png`

### 8.3 写标签

- 用 `metaflac` 写 FLAC Vorbis comment 与 PICTURE（不改音频流）
- 字段：TITLE / ARTIST / ALBUM / ALBUMARTIST / DISCNUMBER / DISCTOTAL / TRACKNUMBER / TRACKTOTAL /
  DATE / LABEL / CATALOGNUMBER / BARCODE / MUSICBRAINZ_ALBUMID / DISCOGS_RELEASE
- 保留 cyanrip 写入的 REPLAYGAIN_* / R128_* / 指纹等（先导出白名单，再 `--remove-all-tags` 后回写）
- 归档路径 `艺人/专辑/`，套装加 `CD1-`/`CD2-` 前缀；目标已存在则报错不覆盖
- 写前校验：源文件必须是完整 FLAC（`fLaC` 头 + 非空），避免搬走还在写的半成品
- 空标题一律回退 `Track NN`，不会产生空白文件名

### 8.4 HDCD

- cyanrip **不做自动检测**，`-H/--hdcd` 默认关闭，日志里的 `HDCD decoding: disabled` 只是开关状态
- 碟片页提供「这张碟是 HDCD（20-bit 解码）」复选框，勾选才传 `-H`
- 勾选后输出 24-bit FLAC（体积约 +50%）；cyanrip 日志会回执
  `HDCD detected: yes, peak_extend: enabled permanently, max_gain_adj: -4.0 dB`
- 已验证：Reference Recordings RR-907CD 实测为 HDCD，解码前后电平差约 6 dB
- 若以后做自动检测：`cdparanoia` 读 60s → `ffmpeg -af "hdcd=disable_autoconvert=0"`；
  **必须带 `disable_autoconvert=0`**，否则滤镜不处理音频却照样打印 `HDCD detected: no`

---

## 9. 错误处理与边界

| 场景 | 处理 |
|---|---|
| 无碟 / 换碟（`No medium found`） | 回到 `NoMedium` 状态，3s 轮询，不弹错误框 |
| 驱动报 media changed | 中止当前碟，保留已完成轨，提示重新放入 |
| 驱动不响应 / 超时 | 中止 + 建议重插/重启驱动，保留日志 |
| 磁盘空间不足 | 开抓前按"轨数 × 轨长 × 176.4KB × 格式系数"预估并校验 |
| 网络失败 | 元数据降级，不阻塞抓轨；结果页可"重试识别" |
| Discogs 限速 | 客户端令牌桶（未认证 25/min，认证 60/min）+ 响应缓存 |
| cyanrip 版本过旧（<0.9.4） | 启动时检测并在设置页警告（已知 media-check 性能 bug） |
| 用户取消 | SIGTERM→SIGKILL，清理残片，标记 PartialDone |
| 输出目录不可写 | 开始前预检 + 明确报错 |

---

## 10. 测试策略

1. **core 单测（离线）**
   - `progress.rs`：喂真实 `\r` 片段，断言事件序列与节流
   - `logparse.rs`：三份真实 log（Quartet 13/13、花儿 CD1 15/16、花儿 CD2 8/8）断言 offset、AR、CRC、错误数
   - `config.rs`：默认值、迁移、损坏文件回退
   - `meta/*`：录制 JSON 快照，断言匹配算法与容差
2. **GUI 冒烟**：`GDK_BACKEND=headless` 或 `weston --headless` 启动，检查页面构建无 panic
3. **真碟验收**（本机）
   - 已知碟：久石譲 Quartet（AR 13/13 conf 8）→ 结果页数字须与命令行一致
   - 未知碟：花儿乐队 CD1（MB/CDDB 均无）→ 走人工匹配路径
4. **回归**：每次发版前跑一遍 `cargo test` + 上述真碟

---

## 11. 里程碑

### M1 — 工程骨架与 core（已完成）
- workspace、crate 划分、CI 脚本（`cargo fmt/clippy/test`）
- `disc.rs`（cdrdao TOC 解析）、`progress.rs`、`logparse.rs` + 三份 fixture 单测
- `rip.rs` 最小实现：启动 cyanrip、收集事件、日志落盘
- 验收：`cargo test` 全绿；CLI 示例可跑完一张碟并打印结构化结果

### M2 — GUI 五页成形（已完成）
- 主窗口、HeaderBar、ViewStack
- 碟片页（设备、碟片信息、曲目表、勾选、快捷参数）
- 设置页（五分类完整实现 + 模板预览 + 参数摘要条）
- 进度页（双进度、速度、日志、取消）
- 结果页（AR 表、CRC、操作按钮）+ 完成提示音（canberra，可关闭）
- i18n 骨架：所有可见字符串走 gettext，初始化 `po/zh_CN.po`
- 验收：真碟（Quartet）全程鼠标操作完成抓轨，结果与命令行一致；切换 `LANG` 不出现空白文案

### M3 — 元数据与标签（已完成，2026-11-07）

**实现**
- `meta::discid`：自行实现 MusicBrainz Disc ID（SHA-1 + libdiscid 的 base64 变体），
  单测直接对照 cyanrip 日志里的真实 ID `kC9g0ZX9IMTELLEHE.ETxsZyaMg-`
- `meta::musicbrainz`：按 discid 查发行版（含曲目时长）
- `meta::discogs`：按条码搜索、取发行版详情（曲目、厂牌、目录号、封面），token 读
  `~/.config/discogs/token` 或 `CYANRIP_DISCOGS_TOKEN`
- `meta::acoustid`：`fpcalc` 指纹 + AcoustID 查询（需 `CYANRIP_ACOUSTID_KEY`）
- `disc::probe` 增加 MCN 读取（`CDROM_GET_MCN`）——很多商业碟的 MCN 就是条码
- 解析顺序：MB(discid) → Discogs(MCN/条码) → AcoustID+Discogs(关键字)，按轨数/时长打分
- 手工兜底：`meta::cue` 解析 CUE 表（UTF-8/GBK 自动识别），取 PERFORMER/TITLE/REM DATE/
  DISCNUMBER/TOTALDISCS/CATALOG 与逐轨 TITLE；`Cue` 作为独立数据源，优先级最高
- `tag`：重命名并归档到 `艺人/专辑/`，`metaflac` 写 Vorbis comment，保留 ReplayGain/AcoustID，
  可内嵌封面；写后可用 `flac -t` 校验
- GUI：结果页新增「元数据」区块——自动识别（`auto_tag_after_rip` 开启时）或手动「识别元数据」，
  下拉选择候选后「写入标签」
- CLI 新增 `match`（列出候选）与 `tag`（写标签归档）

**验收**
- `cyanrip-cli match`（久石譲 Quartet）：DiscID 与 cyanrip 一致；MB 无此压片（404），
  走 MCN 条码命中 Discogs `15601039`，13 轨
- `cyanrip-cli tag`：归档到 `Joe Hisaishi/Quartet (Original Soundtrack)/01 Main Theme.flac`，
  TITLE/ALBUM/TRACKTOTAL=13/DATE/LABEL/UPCH-1105/BARCODE/DISCOGS_RELEASE 全部正确，
  ReplayGain 保留，`flac -t` 通过
- GUI（`CYANRIP_GUI_AUTORIP=1 CYANRIP_GUI_AUTOAPPLY=1`，真机 + 真网络）：
  `META candidates=1 → applied=/tmp/.../Joe Hisaishi/Quartet (Original Soundtrack)`
- 34 个测试全绿，clippy 0 警告

**抓轨前识别（同日补）**
- 识别改为在 `probe` 成功后就触发（只要 TOC + MCN，不需要音频），碟片页即有曲名
- 选中的候选会随 `-a` / `-t` 传给 cyanrip：目录名、文件名、日志里的元数据一次到位，
  抓完只需归档、补 label/barcode/封面
- 介质轮询重复探测同一张碟时保留用户的勾选（之前会重置成全选，导致误抓整张）
- 抓轨前识别未完成时点「开始」，等识别结束再启动（上限 15s），避免丢掉元数据

**套装（多碟）支持（2026-11-07 补）**
- MusicBrainz / Discogs 都按「本碟音轨数」挑出对应的 medium / Discogs 分碟组，
  不再把整套曲目拍平成一条列表（之前 CD2 会拿到 CD1 的曲名）
- 候选带上 `disc_number` / `disc_total`；界面显示 `CD1/2`
- 抓轨时补 `-a disc=N:totaldiscs=M`，并用 `-D "{album} CD{disc} [{format}]"` 分目录，
  两张碟同时抓不会写同一个文件夹
- `tag::apply` 以发行版碟号为准：`DISCNUMBER/DISCTOTAL` 正确，文件名带 `CD1-`/`CD2-`
- 封面除了内嵌进 FLAC，还会在专辑目录里单独存一份 `cover.jpg` / `cover.png`
- 目标文件已存在时仍按「报错不覆盖」处理，避免并发误盖

**并发抓双碟时踩的坑（2026-11-07）**
- 定位「本次抓轨目录」原来只看 mtime/新建差集，两张碟同时抓时会互相认错：
  A 把 B 正在写的 0 字节文件搬到 CD1 目录，metaflac 报 "NOT_A_FLAC_FILE"，
  之后 B 又把 A 的成品搬走，结果两张碟的音频互换
- 现在先用 `DiscID` 匹配目录里的 rip 日志（日志里唯一且稳定），再退回差集/mtime
- `tag::apply` 搬文件前检查 FLAC 文件头（`fLaC` + 非空），不完整的直接报错
- `dirs_with_flac` 只统计写完的 FLAC，0 字节文件不再算数

**已知限制**
- 未识别碟（如花儿 CD2）在 MB 无 discid、无 MCN、无 AcoustID key 时无候选，只能手工补
- 封面优先 Discogs primary 图；MB 候选走 Cover Art Archive

### M4 — 打磨与扩展（进行中）
- ✅ 应用图标与桌面项（app-id 对齐）
- ✅ 多实例（两台光驱并行）+ 独立配置 + 关机互斥
- ✅ 封面（三级兜底、多选刮削器、内嵌 + 目录存图）
- ✅ CUE 导入 / 导出（含封面）
- ✅ HDCD 用户勾选解码
- ⬜ 队列 / 自动换碟 / 失败重试
- ⬜ 取消语义完善、错误页细化
- ⬜ 多语言验证（`po/en.po`）、快捷键
- ⬜ 可选 flatpak 打包

---

## 12. 风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 无法直接观察渲染（本机 GNOME 截图受限：grim 不支持 Mutter，Mutter Screenshot D-Bus 被拒） | UI 细节迭代慢 | 依赖用户截图反馈，或经批准用 computer_use 截取窗口 |
| gtk4-rs 与系统 GTK 4.14 的 API 差异 | 编译失败/功能缺失 | 锁定 crate 版本，先用最小控件集 |
| cyanrip 输出格式随版本变化 | 解析失效 | 解析器对未知行宽容；fixture 覆盖多版本 |
| Discogs 限速/改动 API | 元数据不可用 | 缓存 + 降级到 MB/手工 |
| 老驱动（SiI3811 桥接）异常 | 抓轨中断 | 状态机容错、保留日志、可恢复 |
| Rust 编译时间影响迭代 | 开发慢 | 增量构建；core 单测不依赖 GUI |

---

## 13. 决策记录

2026-11-06 已确认的四个事项，无遗留未决问题：

| 事项 | 决定 |
|---|---|
| 队列 / 多碟 | 保持排在 **M4**，M2 只做单碟 |
| 抓轨以外的自动化 | 暂不考虑，留作以后扩展。**保留"抓完关机"**（默认关）；**新增"抓完播放提示音"开关**，M2 实现（canberra 主题音 / 自定义文件） |
| 界面语言 | 先做**中文**；所有用户可见字符串从 M2 起走 gettext 通道，新增语言只需加 `po/<lang>.po`，M4 用 `en` 验证机制 |
| `ripcd` | **保留**作为过渡工具，M4 的 `cyanrip-cli` 与它并存，不删除 |

---

## 14. 参考资料（本机实测）

- 驱动：`PIONEER DVD-RW DVR-106D 1.00`，经 SiI3811 桥接，`libata.atapi_dmadir=1` 才能枚举
- 读偏移：`+48 samples`（两张碟独立确认，confidence 16 / 13）
- C2：驱动不支持，校验依赖多遍比对
- 速度：裸读 14.8x；cyanrip 0.9.2 因 `CDROM_MEDIA_CHANGED` 每帧查询仅 0.8x；0.9.4-rc3 约 1.5–3.8x
- 一箱参考数据（log fixtures）：Quartet 13 轨/41:22、花儿 CD1 16 轨/46:17、花儿 CD2 8 轨/33:33

### 14.1 GUI 踩坑清单

- **AdwActionRow 默认 `use-markup=true`**：标题里的 `&`（如 `… tenor & orchestra`）会被当 Pango
  标记解析，解析失败整行渲染为空白 → 动态标题必须 `.use_markup(false)`
- **`eject` 会阻塞**：外部命令要等托盘动作，必须放后台线程，否则窗口被判「未响应」
- **桌面项文件名必须等于 app-id**（`io.github.cyanrip.Frontend.desktop`），否则 Wayland 下
  任务栏找不到入口、显示通用图标
- **`Button::from_icon_name` 会加 `image-button` 类**，libadwaita 把它画成无边框；
  要和文字按钮一致得用 `Button::new()` + `Image` + `text-button`
- **Discogs token 失效返回 401**：应退回匿名请求，否则元数据和封面一起失效
- **自测不能写真实配置**：`CYANRIP_GUI_ROOT` 生效时整份配置不落盘；selftest 只在
  `CYANRIP_GUI_CONFIG` 存在时才写假 token
- **ffmpeg hdcd 滤镜默认 `disable_autoconvert=true`**：格式不匹配时它不处理音频，
  却仍打印 `HDCD detected: no`，会误判非 HDCD

---

## 15. 实施进度

### M1 — 已完成（2026-11-07）

- 工具链：rustup + rustc/cargo 1.99.1；`libgtk-4-dev` / `libadwaita-1-dev` 已安装（GTK 4.14.5 / Adw 1.5）
- 代码：`cyanrip-core`（model / event / disc / progress / logparse / rip / config）+ `cyanrip-cli`
- 测试：22 个单测 + 2 个集成测试（假 cyanrip 的流式事件与取消路径），全部通过；`cargo clippy --all-targets` 零警告
- 真机验证：
  - `probe` 在空仓时正确返回"光驱中没有碟片"，不 panic
  - `parse-log` 对三份真实日志输出与 cyanrip 原始结果一致（Quartet 13/13、花儿 CD1 15/16、CD2 8/8）
- 真碟验收（2026-11-07，久石譲 *Quartet*）：
  - `probe`：13 轨 / 41:22 / lead-out 186178，与 cyanrip 一致
  - `rip -l 5 -Z 2`：EAC CRC32 `7FF4B823`、AR v2 `539F7C13`（confidence 8），与 cyanrip 完整抓轨逐位相同
  - 修复：`CDROMREADTOCHDR` 常量误写为 0x5302（实为 `CDROMRESUME`，返回 0 且不写缓冲区），已改为 0x5305
- 待办：GUI crate 随 M2 建立

### M2 — 已完成（2026-11-07）

**基础实现**
- `crates/cyanrip-gui`：AdwApplicationWindow + ToolbarView + HeaderBar + Stack（碟片 / 进度 / 结果 / 设置）
- 碟片页：设备下拉、重新扫描、探测、弹出、测定偏移、曲目勾选
- 设置页：设备与驱动 / 输出 / 元数据（占位）/ 行为（含提示音）/ 高级 / 应用
- 进度页：双进度条 + 终端风格单行状态 + 日志 + 取消
- 结果页：结论 + 概览 + 逐轨明细表
- 工作线程：tokio runtime + `std::sync::mpsc` 回主线程，100ms 轮询，GTK 主线程不阻塞
- i18n：所有字符串经 `gettext`（中文作 msgid），无 `.mo` 时原样回退

**M2 验收后的增补（同日，按使用反馈迭代）**
| 反馈 | 改动 | commit |
|---|---|---|
| 实时日志被进度刷屏 | 进度帧只走 `Progress` 事件；日志只留真实行；新增终端风格单行状态 | `285cf49` |
| 设置页没有保存按钮 | 改为显式保存：状态行 + 保存按钮 + 恢复默认；未保存时按钮可用 | `5ace7d3` |
| 需要全选 | 曲目表加三态"全选"复选框 | `945af66` |
| 结果页信息太少 | 结论句 + 概览键值 + 逐轨 CRC/AR/状态表（带颜色） | `6897c95` |
| 设备框改下拉 | `GtkDropDown` + 型号标签 + 重新扫描；切换即探测 | `c32e897` `fdc3142` |
| 需要弹出按钮 | 碟片页加「弹出」；CLI 加 `eject [--close]` | `4b7685a` |
| 弹出无效 | 桥接层拒绝 `CDROMEJECT`（`0x53/0x02`），改用 util-linux `eject`（SCSI 路径） | `93937a0` |
| 换碟后列表不刷新 | 1.5s 轮询 `CDROM_DRIVE_STATUS`，自动重探 | `5d2fda9` |

**验收**
- `CYANRIP_GUI_SMOKE=1`：窗口构建 + 探测（13 轨 / 41:22）后正常退出
- `CYANRIP_GUI_AUTORIP=1`：界面驱动的单轨抓轨 `1/1` 精确命中，CRC `01834C83`、AR `17AF569B`（与命令行逐位一致），结果页构建出 4 个区块
- `CYANRIP_GUI_SELFTEST=1`：保存流程（初始置灰 → 改动变脏 → 保存写盘 → 回到干净）+ 全选/取消全选 + 设备下拉（`devices=1 selected=/dev/sr0`）
- `CYANRIP_GUI_EJECTTEST=1`：点击"弹出"后 `probe` 报"光驱托盘已打开"，自动收回成功
- 换碟轮询实测：`Ok → TrayOpen → Ok`，每次回到 `Ok` 都触发 `auto_probe=true tracks=13`
- 27 测试全绿，clippy 0 警告

**踩坑**
- AdwApplicationWindow 不支持 `set_titlebar`（必须用 `adw::ToolbarView` 承载 HeaderBar）
- 探测时不能把未知偏移 `0` 写进配置，否则覆盖已测好的 +48
- `CDROMREADTOCHDR` 是 `0x5305`（`0x5302` 是 `CDROMRESUME`，返回 0 且不写缓冲区）
- 本机经 SiI3811 桥接：`CDROMEJECT` 被 `MEDIUM REMOVAL PREVENTED` 拒绝，且 ioctl 返回 0（假成功），必须走 SCSI 路径

### M3 — 元数据与标签（已完成，2026-11-07）

**实现**
- `meta::discid`：自行实现 MusicBrainz Disc ID，单测对照真值 `kC9g0ZX9IMTELLEHE.ETxsZyaMg-`
- `meta::musicbrainz` / `meta::discogs` / `meta::acoustid` / `meta::cue` / `meta::google`
- `disc::probe` 增加 MCN 读取（`CDROM_GET_MCN`），商业碟的 MCN 往往就是条码
- `tag`：重命名 + 归档 + `metaflac` 写标签 + 内嵌封面 + `cover.jpg` 落盘
- GUI：结果页「元数据」区块；CLI：`match` / `tag`

**验收（真碟）**
- Quartet：MB 无此压片 → MCN 条码命中 Discogs `15601039`，13 轨
- 归档 `Joe Hisaishi/Quartet (Original Soundtrack)/01 Main Theme.flac`，标签与 ReplayGain 正确，`flac -t` 通过
- 抓轨前识别（`META candidates=1` 早于 `RIP`），参数里带完整 `-a/-t`

### M3.5 — 使用反馈驱动的增补（2026-11-07，已完成）

| 反馈 / 需求 | 实现 | commit |
|---|---|---|
| 识别要在抓轨前，曲目表直接显示曲名 | 探测成功即后台识别；`track_titles` 注入列表；未识别完点抓轨会等待 | `25259a3` |
| 「探测」和「重新识别」重复 | 删除「重新识别」，探测即识别 | `df8f54a` |
| 「探测」和「重新扫描」重复 | 合并为「刷新」= 重扫设备 + 读 TOC + 识别 | `36e5d55` |
| 弹出按钮用通用图标 | `media-eject-symbolic`；`from_icon_name` 会带 `image-button` 类导致无边框，改为 `Button::new` + `Image` + `text-button` | `6281c34` `90278d7` `a53e884` |
| 看不到当前跑的是哪版 | 设置 → 高级 显示「程序构建时间」 | `32e617b` |
| 文本类设置要能选 | 输出根目录改目录选择器；提示音改事件下拉（36 个）+ 试听 | `d24419f` |
| 双光驱同时抓 | 独立配置 `CYANRIP_GUI_CONFIG` + `cyanrip-frontend-2` 启动器；实例锁；关机互斥 | `06ae895` |
| 双碟套装会串轨/撞文件 | 按音轨数挑 medium / Discogs 分碟组；`-a disc=…` + `-D "{album} CD{disc}"`；`DISCNUMBER` 正确 | `8fd34bc` |
| 设置里要能填 Discogs token | 掩码输入，存 `secrets.toml` 0600；旧 `~/.config/discogs/token` 自动回填 | `79308a5` |
| 数据库里没有的碟要能手填曲目 | CUE 导入（UTF-8/GBK），作为最高优先候选 | `84d5b42` |
| 碟片页要能看封面 | 封面预览 + 三级取图链 + 缓存 | `66a7f89` `b8dc13c` |
| 封面兜底要能选来源 | 三个刮削器复选框，全不勾则不刮 | `dd6b9c9` |
| 要能导出 CUE 和封面 | 单镜像 CUE（含 INDEX 时间）+ `<专辑>.jpg` | `3f5080f` |
| 任务栏图标与启动器不一致 | 新图标 + 桌面项改名 = app-id + `set_icon_name` | `726b48d` |
| 图标样式（RIP 字样） | 多轮调整：RIP 叠在盘面下缘、灰色、白描边 | `a4df532` `f7ebcb0` `f9d1648` |
| 封面没跟着音频存 | 归档时额外写 `cover.jpg` / `cover.png` | `555b804` |
| 「开始抓轨」要显眼 | 移到碟片页、320×64、1.35em、居中，最终放在候选下拉框正下方 | `8e07815` `1613068` |
| 弹出后假死 + 信息残留 | `eject` 改后台线程；`clear_disc_ui` 统一清空；过期识别结果丢弃 | `d88ee17` |
| 部分曲名空白 | `AdwActionRow` 默认 `use-markup=true`，`&` 解析失败导致空白 → 显式关闭 markup | `4b9efb4` |
| HDCD 无法自动判断 | 碟片页加「这张碟是 HDCD」复选框，勾选才传 `-H`（输出 24-bit） | `40fc42f` |
| Discogs token 失效导致无封面 | 401 时自动匿名重试；自测不再污染真实配置 | `ce4cf0b` |

**验证基线**：40 单测 + 2 集成测试 + 1 个 GUI 参数拼装测试；clippy 0 警告；
真碟并发（sr0/sr1 双碟套装）、CUE 导入导出、封面内嵌、HDCD 24-bit 均已实测。

### M4 — 剩余工作

- 队列 / 自动换碟 / 失败重试
- 取消语义完善、错误页细化
- 多语言验证（`po/en.po`）、快捷键
- 可选 flatpak 打包
- 可选：HDCD 自动检测（读 60s + ffmpeg 分析，命中自动勾选）
---

---

## 16. 自测开关（GUI）

| 环境变量 | 作用 |
|---|---|
| `CYANRIP_GUI_SMOKE=1` | 构建窗口并探测碟片，2 秒后退出（验证不崩） |
| `CYANRIP_GUI_AUTORIP=1` | 只抓第一轨，完成后打印 `AUTORIP_OK n/m` 与结果页区块数并退出 |
| `CYANRIP_GUI_SELFTEST=1` | 保存流程 + 全选 + 设备下拉断言，打印 `SELFTEST ...` |
| `CYANRIP_GUI_EJECTTEST=1` | 模拟点击"弹出"，检查托盘状态并自动收回 |
| `CYANRIP_GUI_STANDALONE=1` | 以 NON_UNIQUE 方式启动，允许与已开实例并存 |
| `CYANRIP_GUI_DEBUG=1` | 打印介质跳变、识别结果、封面/导出步骤 |
| `CYANRIP_GUI_CONFIG=<path>` | 使用独立配置文件（多开/测试；配套 secrets 同名） |
| `CYANRIP_GUI_ROOT=<dir>` | 临时覆盖输出根目录，且**不写回配置**（测试用） |
| `CYANRIP_GUI_AUTOAPPLY=1` | 抓完自动写标签（与 AUTORIP 搭配做端到端） |
| `CYANRIP_GUI_FAKE_META=1` | 用合成候选替代联网识别（只测 UI → 打标签链路） |
| `CYANRIP_GUI_IMPORT_CUE=<file>` | 启动即导入指定 CUE 并打印前 3 轨标题 |
| `CYANRIP_GUI_EXPORT_DIR=<dir>` | 启动即导出 CUE + 封面到指定目录 |
| `CYANRIP_DISCOGS_TOKEN` / `CYANRIP_ACOUSTID_KEY` | 元数据源凭据（优先级高于界面设置） |

用法示例：

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0
CYANRIP_GUI_STANDALONE=1 CYANRIP_GUI_SELFTEST=1 ./target/debug/cyanrip-frontend
```

---

## 17. 变更日志（按提交）

| commit | 日期 | 说明 |
|---|---|---|
| `40fc42f` | 11-07 | disc page: user-controlled HDCD decoding checkbox (-H) |
| `4b9efb4` | 11-07 | ui: disable markup on track rows (ampersand titles rendered blank) |
| `d88ee17` | 11-07 | eject: run off the main thread and clear all disc UI state |
| `1613068` | 11-07 | ui: place start-rip right under the album dropdown |
| `c480354` | 11-07 | clippy: compare type name without allocation |
| `8e07815` | 11-07 | ui: move start-rip into disc page as large centered button |
| `555b804` | 11-07 | tagging: also save cover.jpg alongside audio |
| `ce4cf0b` | 11-07 | discogs: retry anonymously on 401; keep selftest token out of real config |
| `f9d1648` | 11-07 | icon: bigger RIP overlapping disc edge |
| `f7ebcb0` | 11-07 | icon: larger centered disc, gray RIP overlay |
| `a4df532` | 11-07 | icon: replace rip arrow with RIP lettering |
| `726b48d` | 11-07 | app icon: ship matching icon and app-id desktop entry |
| `3f5080f` | 11-07 | disc page: export CUE + cover image |
| `dd6b9c9` | 11-07 | settings: multi-select cover scrapers (none = no cover art) |
| `b8dc13c` | 11-07 | cover: add Google image search fallback (API key + CX) |
| `66a7f89` | 11-07 | disc page: fetch and show cover art with Discogs fallback |
| `f1b4208` | 11-07 | clippy: collapse consecutive replace in cue parser |
| `84d5b42` | 11-07 | disc page: import track info from CUE (UTF-8/GBK) as top-priority candidate |
| `79308a5` | 11-07 | settings: Discogs access token field stored in secrets.toml (0600) |
| `8fd34bc` | 11-07 | multi-disc: pick medium per disc, disc-aware folders/tags, exact rip-dir by DiscID |
| `06ae895` | 11-07 | support parallel instances: isolated config, per-instance lock, exact rip-dir pick |
| `d24419f` | 11-07 | settings: directory picker for output root, sound event dropdown with preview |
| `32e617b` | 11-07 | ui: show build time in settings to spot stale windows |
| `a53e884` | 11-07 | ui: give eject icon button the same styling as text buttons |
| `90278d7` | 11-07 | ui: drop flat style so eject button matches other buttons |
| `6281c34` | 11-07 | ui: use media-eject icon for the eject button |
| `36e5d55` | 11-07 | ui: merge rescan and probe into a single refresh button |
| `df8f54a` | 11-07 | ui: drop redundant re-identify button (probe already identifies) |
| `b1981de` | 11-07 | docs: describe pre-rip identification |
| `25259a3` | 11-07 | M3: identify metadata before rip and show track titles on disc page |
| `d1350e6` | 11-07 | M3: metadata (discid, MB, Discogs, AcoustID) and FLAC tagging with GUI/CLI |
| `c0392bb` | 11-07 | docs: fold M2 follow-up changes into plan and README |
| `5d2fda9` | 11-07 | gui: poll media status every 1.5s and auto-refresh track list on disc change |
| `93937a0` | 11-07 | fix eject: CDROMEJECT is rejected on the SiI3811 bridge, fall back to util-linux eject (SCSI) |
| `4b7685a` | 11-07 | gui/cli: eject button and eject/close-tray commands |
| `fdc3142` | 11-07 | gui: wire device dropdown selection and rescan button |
| `c32e897` | 11-07 | gui: device picker as dropdown with model labels and rescan |
| `6897c95` | 11-07 | gui: richer result page — summary grid, verdict, per-track table |
| `945af66` | 11-07 | gui: select all / deselect all checkbox with tri-state sync |
| `5ace7d3` | 11-07 | gui: explicit save button on settings page with dirty state and restore-defaults |
| `285cf49` | 11-07 | gui: keep progress out of the log pane, show one terminal-style status line |
| `d4c6126` | 11-07 | M2: GTK4 GUI (disc/progress/result/settings pages), offset detection, e2e autorip verified |
| `47ae591` | 11-07 | M1 verified on real disc: fix CDROMREADTOCHDR ioctl, monotonic multi-pass progress, sane speed/ETA |
| `cd8423a` | 11-07 | M1: cyanrip-core + cyanrip-cli (TOC probe, progress/log parsing, rip orchestration) |

> 更详细的功能验收记录见 §15（M1/M2/M3/M3.5），踩坑见 §14.1。
