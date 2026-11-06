# cyanrip-frontend

cyanrip 的原生 GTK4 前端（Rust）。设计文档见 [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md)。

当前状态：**M1 完成**（core + 无头 CLI，GUI 未开始）。

## 结构

```
crates/cyanrip-core/   # 碟片探测、抓轨编排、进度与日志解析（不依赖 GTK）
crates/cyanrip-cli/    # 无头入口：probe / parse-log / rip
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
