# cyanrip-frontend

cyanrip 的原生 GTK4 前端（Rust）。设计文档见 [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md)。

当前状态：**M2 完成**（core + CLI + GTK4 五页 GUI）。

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
```

界面：顶部切换「碟片 / 进度 / 结果 / 设置」。碟片页填设备与偏移 → 探测 → 勾选曲目 → 开始抓轨；
抓轨中显示本轨与总进度、倍速、已用/剩余、实时日志；完成后结果页列出每轨 EAC CRC32 与 AR 状态。

自测开关（不需要人工点界面）：

```bash
CYANRIP_GUI_SMOKE=1   timeout 20 ./target/debug/cyanrip-frontend   # 只构建并退出
CYANRIP_GUI_AUTORIP=1 timeout 400 ./target/debug/cyanrip-frontend  # 只抓第一轨，打印结果后退出
```

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
