# cyanrip-frontend

cyanrip 的原生 GTK4 前端（Rust）。**中文** · [English](README.en.md)

设计文档见 [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md)。

当前状态：**v1.0**（2026-11-08）。M1–M4 完成并通过真碟验收。

## 结构

```
crates/cyanrip-core/   # 碟片探测、抓轨编排、进度与日志解析（不依赖 GTK）
crates/cyanrip-cli/    # 无头入口：probe / parse-log / rip
crates/cyanrip-gui/    # GTK4 + libadwaita 前端（碟片 / 进度 / 结果 / 设置）
```

## 依赖

- Rust stable（rustup）
- 运行时：`cyanrip`（≥ 0.9.4）、`cdrdao`（M3 读 ISRC 用）、`fpcalc`（M3）
- 构建 GUI 时还需要：`libgtk-4-dev`、`libadwaita-1-dev`

## 构建与测试

```bash
cargo build
cargo test          # 53 个测试（含流式/取消、归档模板、i18n 覆盖），全部离线
```

## 双光驱同时抓轨

默认单实例。要用两台光驱并行抓轨，用第二个入口（独立配置、允许并存）：

```bash
cyanrip-frontend-2     # = CYANRIP_GUI_STANDALONE=1 + CYANRIP_GUI_CONFIG=~/.config/cyanrip-frontend/second.toml
```

两边各自选光驱即可；设备/偏移/输出目录互不影响，抓完关机也会等最后一个实例结束。

**双碟套装**可以两台光驱同时抓：识别会按音轨数挑出各自那张碟的曲目（不会串成 CD1 的曲名），
目录自动分成 `专辑 CD1 [FLAC]` / `专辑 CD2 [FLAC]`，标签写 `DISCNUMBER/DISCTOTAL`，
文件名带 `CD1-`/`CD2-` 前缀，可以放进同一个输出根目录。

同一张专辑、同一碟号重复抓仍会因目标文件已存在而报错（不会覆盖）。

## 启动 GUI

```bash
cargo run -p cyanrip-gui            # 调试
cargo build --release -p cyanrip-gui && ./target/release/cyanrip-frontend
# 安装（含图标与桌面项，供任务栏/应用菜单使用）
install -Dm755 target/release/cyanrip-frontend ~/.local/bin/cyanrip-frontend
install -Dm644 assets/io.github.cyanrip.Frontend.svg \
        ~/.local/share/icons/hicolor/scalable/apps/io.github.cyanrip.Frontend.svg
install -Dm644 assets/io.github.cyanrip.Frontend.desktop \
        ~/.local/share/applications/io.github.cyanrip.Frontend.desktop
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor

# 或直接运行已安装版本
~/.local/bin/cyanrip-frontend
```

界面：顶部切换「碟片 / 进度 / 结果 / 设置」。

- **碟片**：设备下拉（带型号，切换即探测）、刷新（重扫光驱 + 读 TOC + 识别）、弹出、读偏移 + 测定偏移、全选/取消全选、逐轨勾选；换碟后 1.5s 内自动刷新列表；专辑候选下拉框正下方有「这张碟是 HDCD」复选框和大号「开始抓轨」按钮（勾选 HDCD 会传 `-H`，输出 24-bit FLAC）
- **封面**：碟片页左侧显示当前候选的封面，取图顺序 MB(CAA) → Discogs → Google 自定义搜索（可选，需在设置里填 API Key + CX）；设置 → 元数据 里可勾选用哪些刮削器，全不勾就完全不下图、也不写进 FLAC；可点「取得封面」重取，抓轨归档时复用这张图，除了写进 FLAC 还会在专辑目录里存一份 `cover.jpg`
- **抓轨前识别**：探测成功即查 MusicBrainz / Discogs（用 Disc ID + MCN，不需要音频），曲目表直接显示真实曲名；候选下拉可切换压片版本，选中的元数据随 `-a`/`-t` 传给 cyanrip，目录名与文件名一次到位
- **导出 CUE + 封面**：把当前曲目信息导出成 CUE（含逐轨 INDEX 时间）和封面图，方便备份/交给别的工具
- **导入曲目信息**：数据库里查不到的碟（尤其是中文碟），可导入 CUE 表（UTF-8/GBK 均可），曲目名/专辑/年份/碟号直接进列表，抓轨与归档都用它
- **进度**：双进度条 + 终端风格单行状态（`Ripping track 5, progress - 47.32%, ETA - 2m · 2.3x · 已用 12:40`）；日志只保留真实输出行
- **结果**：结论句 + 概览（设备/偏移/C2/AR ID/错误数…）+ 逐轨 EAC CRC32 / AR v1 / AR v2 / confidence / 状态表
- **结果页「元数据」**：抓轨后自动识别（MusicBrainz → Discogs 条码/MCN → AcoustID），下拉选候选后「写入标签」，自动归档到 `艺人/专辑/` 并写 FLAC 标签
- **设置**：设备与驱动 / 输出（根目录用目录选择器）/ 元数据（Discogs token 掩码输入，存 `secrets.toml` 0600）/ 行为（提示音下拉 + 试听、托盘、通知、关机）/ 高级（含构建时间）/ **应用**（显式保存 + 恢复默认）

## 归档模板

设置 → 输出 里可改目录/文件/日志模板，归档阶段生效：

- 字段：`{artist}` `{album}` `{year}` `{title}` `{track}` `{track:02}` `{disc}` `{disc_total}` `{track_total}`
- 默认：目录 `{artist}/{album}`、文件 `{track:02} {title}`、日志 `{album}`
- 套装且文件名模板不含 `{disc}` 时自动补 `CDn-` 前缀
- 值会做文件名清理；目录里出现 `..` 直接拒绝

## 快捷键

`Ctrl+R` 开始抓轨 · `Ctrl+E` 弹出 · `F5` 刷新 · `Ctrl+Q` 退出

## 界面语言

设置 → 行为 → **界面语言** 可选「跟随系统 / 中文 / English」，写入 `ui.language`，**重启程序生效**。

默认中文（msgid 即中文）。`po/en.po` 是英文翻译，编译后放到
`po/en/LC_MESSAGES/cyanrip-frontend.mo` 或 `~/.local/share/locale/en/LC_MESSAGES/`：

```bash
po/update.sh          # 重新提取 msgid + 合并 + 编译 + 安装 .mo
```

改了界面文案就重跑一次；`en.po` 里留空的条目自动回落到中文。

## 自测开关

| 变量 | 作用 |
|---|---|
| `CYANRIP_GUI_SMOKE=1` | 构建 + 探测后 2 秒退出 |
| `CYANRIP_GUI_AUTORIP=1` | 界面驱动单轨抓轨，打印 `AUTORIP_OK n/m` |
| `CYANRIP_GUI_SELFTEST=1` | 保存流程 / 全选 / 设备下拉断言 |
| `CYANRIP_GUI_EJECTTEST=1` | 点击"弹出"并检查托盘状态，自动收回 |
| `CYANRIP_GUI_STANDALONE=1` | 允许多实例（与已开窗口并存） |
| `CYANRIP_GUI_DEBUG=1` | 打印介质状态跳变与元数据进度 |
| `CYANRIP_GUI_AUTOAPPLY=1` | 识别到候选后自动写标签（测试用） |
| `CYANRIP_GUI_FAKE_META=1` | 用合成候选替代联网识别（只测 UI 链路） |
| `CYANRIP_GUI_ROOT=<dir>` | 临时覆盖输出根目录，避免污染音乐库 |
| `CYANRIP_DISCOGS_TOKEN` / `CYANRIP_ACOUSTID_KEY` | 元数据源凭据（Discogs 也可用 `~/.config/discogs/token`） |

## 用法

```bash
# 探测碟片（读 TOC，不抓轨）
cargo run -p cyanrip-cli -- probe --device /dev/sr0

# 解析已有的 EAC 风格日志
cargo run -p cyanrip-cli -- parse-log tests/fixtures/log_13track.txt

# 抓轨（透传 cyanrip 参数）
cargo run -p cyanrip-cli -- rip --device /dev/sr0 --offset 48 -- -Z 2 -P 3 -o flac -N
```

## 已验证

- 三份真实日志 fixture（单碟 13 轨 13/13、套装 CD1 15/16、CD2 8/8）解析结果与 cyanrip 输出一致
- 假 cyanrip 脚本驱动的流式事件：进度、TrackStarted、TrackDone、RipFinished
- 取消路径：`watch` 通道触发后 300ms 内终止子进程并返回 `Cancelled`
- **真碟**（13 轨测试碟，PIONEER DVR-106D，offset +48）：
  - `probe` 读出 13 轨 / 41:22 / lead-out 186178，与 cyanrip 日志一致
  - `rip -l 5 -Z 2` 产出 `EAC CRC32 7FF4B823`、`AR v2 539F7C13 (confidence 8)`，与之前 cyanrip 完整抓轨的结果逐位相同
  - `-Z 2` 三遍读取时总进度单调递增，倍速读数 1.8–3.4x

## 已知坑

- `CDROMREADTOCHDR` 是 **0x5305**，不是 0x5302（0x5302 是 `CDROMRESUME`）。写错时 ioctl 返回 0、缓冲区一个字节都不写，属于静默失败——本项目踩过一次。
- 全碟进度按**实际勾选的轨**折算；cyanrip 自带的 ETA 是滑动窗口估算，开头几秒可能很离谱，程序会自己重算。
- 本机经 SiI3811 桥接：`CDROMEJECT` ioctl 会被拒绝（内核日志 `0x53/0x02 MEDIUM REMOVAL PREVENTED`）且返回 0（假成功），必须走 util-linux `eject`（SCSI `START STOP UNIT`）路径。
- `AdwApplicationWindow` 不支持 `set_titlebar`，标题栏要用 `adw::ToolbarView` 承载。

## 许可证

GPL-3.0-or-later，见 [LICENSE](LICENSE)。

`cyanrip` 本身是 GPL-2.0-or-later 的独立程序，本项目通过子进程调用它，不链接其代码。
