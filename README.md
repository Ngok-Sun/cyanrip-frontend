# cyanrip-frontend

cyanrip 的原生 GTK4 前端（Rust）。设计文档见 [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md)。

当前状态：**M2 完成**（core + CLI + GTK4 五页 GUI，已通过真碟验收）。下一步 M3：元数据与标签。

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
cargo test          # 22 个单测 + 2 个流式/取消集成测试，全部离线
```

## 启动 GUI

```bash
cargo run -p cyanrip-gui            # 调试
cargo build --release -p cyanrip-gui && ./target/release/cyanrip-frontend
# 或直接运行已安装版本
~/.local/bin/cyanrip-frontend
```

界面：顶部切换「碟片 / 进度 / 结果 / 设置」。

- **碟片**：设备下拉（带型号，切换即探测）、重新扫描、探测、弹出、读偏移 + 测定偏移、全选/取消全选、逐轨勾选；换碟后 1.5s 内自动刷新列表
- **进度**：双进度条 + 终端风格单行状态（`Ripping track 5, progress - 47.32%, ETA - 2m · 2.3x · 已用 12:40`）；日志只保留真实输出行
- **结果**：结论句 + 概览（设备/偏移/C2/AR ID/错误数…）+ 逐轨 EAC CRC32 / AR v1 / AR v2 / confidence / 状态表
- **设置**：设备与驱动 / 输出 / 元数据（M3 占位）/ 行为（提示音、托盘、通知、关机）/ 高级 / **应用**（显式保存 + 恢复默认）

## 自测开关

| 变量 | 作用 |
|---|---|
| `CYANRIP_GUI_SMOKE=1` | 构建 + 探测后 2 秒退出 |
| `CYANRIP_GUI_AUTORIP=1` | 界面驱动单轨抓轨，打印 `AUTORIP_OK n/m` |
| `CYANRIP_GUI_SELFTEST=1` | 保存流程 / 全选 / 设备下拉断言 |
| `CYANRIP_GUI_EJECTTEST=1` | 点击"弹出"并检查托盘状态，自动收回 |
| `CYANRIP_GUI_STANDALONE=1` | 允许多实例（与已开窗口并存） |
| `CYANRIP_GUI_DEBUG=1` | 打印介质状态跳变 |

## 用法

```bash
# 探测碟片（读 TOC，不抓轨）
cargo run -p cyanrip-cli -- probe --device /dev/sr0

# 解析已有的 EAC 风格日志
cargo run -p cyanrip-cli -- parse-log tests/fixtures/log_quartet.txt

# 抓轨（透传 cyanrip 参数）
cargo run -p cyanrip-cli -- rip --device /dev/sr0 --offset 48 -- -Z 2 -P 3 -o flac -N
```

## 已验证

- 三份真实日志 fixture（久石譲 *Quartet* 13/13、花儿乐队 CD1 15/16、CD2 8/8）解析结果与 cyanrip 输出一致
- 假 cyanrip 脚本驱动的流式事件：进度、TrackStarted、TrackDone、RipFinished
- 取消路径：`watch` 通道触发后 300ms 内终止子进程并返回 `Cancelled`
- **真碟**（久石譲 *Quartet*，PIONEER DVR-106D，offset +48）：
  - `probe` 读出 13 轨 / 41:22 / lead-out 186178，与 cyanrip 日志一致
  - `rip -l 5 -Z 2` 产出 `EAC CRC32 7FF4B823`、`AR v2 539F7C13 (confidence 8)`，与之前 cyanrip 完整抓轨的结果逐位相同
  - `-Z 2` 三遍读取时总进度单调递增，倍速读数 1.8–3.4x

## 已知坑

- `CDROMREADTOCHDR` 是 **0x5305**，不是 0x5302（0x5302 是 `CDROMRESUME`）。写错时 ioctl 返回 0、缓冲区一个字节都不写，属于静默失败——本项目踩过一次。
- 全碟进度依赖 TOC 的轨长；用 `-l` 只抓部分轨时，ETA 仍按整碟估算。
- 本机经 SiI3811 桥接：`CDROMEJECT` ioctl 会被拒绝（内核日志 `0x53/0x02 MEDIUM REMOVAL PREVENTED`）且返回 0（假成功），必须走 util-linux `eject`（SCSI `START STOP UNIT`）路径。
- `AdwApplicationWindow` 不支持 `set_titlebar`，标题栏要用 `adw::ToolbarView` 承载。
