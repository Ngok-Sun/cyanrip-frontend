# cyanrip-frontend 开发计划

> 状态：M1（core + CLI）与 M2（GUI 五页）已完成并通过真碟验收，下一步 M3（元数据与标签）
> 实施进度见 §15，自测开关见 §16
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
| metaflac / flac | 1.4+ | 1.4.3 ✅（验证用，标签走 lofty） |
| canberra-gtk-play | 任意（提示音） | ✅ 已安装；回退 `pw-play` |

安装命令：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
sudo apt install -y libgtk-4-dev libadwaita-1-dev build-essential pkg-config
```

> 注意：本机 shell 的 `python3` 指向 Hermes venv，无 `gi`；GUI 不用 Python，仅记录以免混淆。

### 2.2 Rust crates

- GUI：`gtk4`、`libadwaita`（`gtk4` crate 需与系统 GTK 版本匹配，0.9.x 支持 4.10+）
- 异步：`tokio`（process、io-util、sync、time）、`async-channel`
- 序列化：`serde`、`serde_json`、`toml`
- 网络：`reqwest`（rustls，避免 OpenSSL 版本纠缠）
- 标签：`lofty`（FLAC Vorbis comment + picture 读写）
- 错误：`thiserror`（库层）、`anyhow`（应用层）
- 日志：`tracing`、`tracing-subscriber`
- 其他：`directories`、`chrono`、`urlencoding`、`regex`、`unicode-normalization`
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
   │     tag.rs                   # FLAC 标签与封面写入（lofty）
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
（AdwApplicationWindow 不支持 `set_titlebar`，必须用 ToolbarView）。顶部 StackSwitcher 切页，右上角"开始抓轨"。

```
┌────────────────────────────────────────────────────────────┐
│ 碟片 │ 进度 │ 结果 │ 设置                    [开始抓轨 ▸]  │
├────────────────────────────────────────────────────────────┤
│ 设备 [/dev/sr0 · PIONEER DVD-RW DVR-106D ▾] [重新扫描] [探测] [弹出] │
│ 偏移 [48] [测定偏移]                                        │
│ PIONEER DVD-RW DVR-106D 1.00 · 13 轨 · 41:22                │
│ ☑ 全选                                                      │
│ ┌────────────────────────────────────────────────────────┐ │
│ │ ☑  01  未命名                                   3:18    │ │
│ │ ☑  02  未命名                                   3:29    │ │
│ │ ☐  03  未命名                                   2:08    │ │
│ └────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────┘
```

- 设备为 `GtkDropDown`，项来自 `/dev/sr*`，标签带 sysfs 读到的厂商+型号；切换即自动探测
- 「重新扫描」重建列表并保留选择；「弹出」调 `eject`（见 §14 桥接坑）
- 「测定偏移」调 `cyanrip -f`，结果按光驱型号记忆
- 「全选」为三态复选框：逐轨取消时自动变半选
- 曲目表用 `GtkListBox` + `AdwActionRow`（13–24 轨规模足够，未用 ColumnView）
- **介质轮询**：每 1.5 秒查 `CDROM_DRIVE_STATUS`，换碟/收回托盘后自动重新探测并刷新列表；抓轨期间暂停轮询

### 7.2 设置页（五分类，独立页面而非弹窗）

左侧分类列表，右侧 `AdwPreferencesPage` 滚动表单：

1. **设备与驱动**：设备、偏移（+自动检测）、速度、Paranoia、重试、`-Z`、overread、C2 说明
2. **输出**：根目录、四个模板（含实时预览）、格式多选、image 模式、CUE/log/封面/ReplayGain/HDCD/de-emphasis/数据轨
3. **元数据**：源优先级（可拖动）、Discogs token（掩码，0600 存储）、语言、封面来源与尺寸、标签字段映射
4. **行为**：自动打标签、抓完弹托盘、桌面通知、**抓完播放提示音（开关 + 主题音/自定义文件）**、抓完关机（默认关，开启需二次确认）、出错策略、未识别碟处理
5. **高级/诊断**：cyanrip 可执行路径与版本探测（区分 `/usr/local/bin` 与 `/usr/bin`）、额外参数透传、日志目录、调试级别、导出诊断包、**界面语言（当前仅中文）**
6. **应用**：设置状态行 + **保存**按钮（无改动时置灰）、**恢复默认**（只改内存，仍需保存）

交互规则（实现后修正）：

- **显式保存**：改动只标记为"未保存"（状态行 + 保存按钮可用），点保存才原子写 `config.toml`；探测/换碟不会顺手把未保存改动写盘
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

## 8. 元数据与标签

### 8.1 识别链路

```
碟片 TOC / discid
  ├─ 1. MusicBrainz discid lookup            （最快，命中即有完整曲目）
  ├─ 2. AcoustID 指纹（fpcalc 取 1–2 轨）     （MB 未命中时）
  ├─ 3. Discogs：barcode / catalogno → release（确定压片版本与封面）
  └─ 4. 失败：占位命名 + 结果页人工指定
```

实测经验：

- ISRC 不是 Discogs 的检索字段；只可用 MB `/ws/2/isrc/<code>` 做交叉校验
- Discogs 时长常被四舍五入到整分秒，匹配容差取 ±4s
- 中文碟常无 ISRC/CD-TEXT，指纹（Shazam/AcoustID）是有效兜底

### 8.2 写标签

- `lofty` 直接写 FLAC Vorbis comment 与 PICTURE，不重编码
- 字段：TITLE / ARTIST / ALBUM / ALBUMARTIST / DISCNUMBER / DISCTOTAL / TRACKNUMBER / TRACKTOTAL / DATE / LABEL / CATALOGNUMBER / BARCODE / GENRE / MUSICBRAINZ_ALBUMID / DISCOGS_RELEASE
- 保留 cyanrip 写入的 REPLAYGAIN_* / R128_* / AcoustID 指纹
- 写后自检：重新读取标签 + 调用 `flac -t` 校验流完整性（可选，结果页显示）

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

### M1 — 工程骨架与 core（预计 1 天）
- workspace、crate 划分、CI 脚本（`cargo fmt/clippy/test`）
- `disc.rs`（cdrdao TOC 解析）、`progress.rs`、`logparse.rs` + 三份 fixture 单测
- `rip.rs` 最小实现：启动 cyanrip、收集事件、日志落盘
- 验收：`cargo test` 全绿；CLI 示例可跑完一张碟并打印结构化结果

### M2 — GUI 五页成形（预计 2 天）
- 主窗口、HeaderBar、ViewStack
- 碟片页（设备、碟片信息、曲目表、勾选、快捷参数）
- 设置页（五分类完整实现 + 模板预览 + 参数摘要条）
- 进度页（双进度、速度、日志、取消）
- 结果页（AR 表、CRC、操作按钮）+ 完成提示音（canberra，可关闭）
- i18n 骨架：所有可见字符串走 gettext，初始化 `po/zh_CN.po`
- 验收：真碟（Quartet）全程鼠标操作完成抓轨，结果与命令行一致；切换 `LANG` 不出现空白文案

### M3 — 元数据与标签（预计 1.5 天）
- MB → AcoustID → Discogs 三级链路 + 缓存 + 限速
- 元数据页（候选选择、映射预览、手工指定）
- `lofty` 标签写入 + 封面内嵌 + 写后校验
- 验收：Quartet 自动带标签归档；花儿 CD1 走人工匹配后正确归档

### M4 — 打磨与扩展（预计 1–2 天）
- 队列/多碟、自动换碟、失败重试
- 取消语义完善、错误页细化
- 多语言机制验证：新增 `po/en.po`（不随包发布，仅验证不改代码即可加语言）、快捷键、应用图标
- `cyanrip-cli`（与 `ripcd` **并存**，`ripcd` 作为轻量无头工具保留，不删除）
- 可选 flatpak 打包

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

### M3 — 元数据与标签（预计 1.5 天）
- MB → AcoustID → Discogs 三级链路 + 缓存 + 限速
- 元数据页（候选选择、映射预览、手工指定）
- `lofty` 标签写入 + 封面内嵌 + 写后校验
- 验收：Quartet 自动带标签归档；花儿 CD1 走人工匹配后正确归档

### M4 — 打磨与扩展（预计 1–2 天）
- 队列/多碟、自动换碟、失败重试
- 取消语义完善、错误页细化
- 多语言机制验证：新增 `po/en.po`（不随包发布，仅验证不改代码即可加语言）、快捷键、应用图标
- `cyanrip-cli`（与 `ripcd` **并存**，`ripcd` 作为轻量无头工具保留，不删除）
- 可选 flatpak 打包

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
| `CYANRIP_GUI_DEBUG=1` | 打印介质状态跳变与自动重探结果 |

用法示例：

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0
CYANRIP_GUI_STANDALONE=1 CYANRIP_GUI_SELFTEST=1 ./target/debug/cyanrip-frontend
```
