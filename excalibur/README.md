# Excalibur CLI

**参数选择器。** 凡是「命令我会写，但那个*参数*我记不住、shell 也补全不了」的场合，
给一个键盘驱动的选择器，选完把完整命令吐回 shell。基于 Rust + ratatui。

| 模块 | 选的参数 | 入口 |
|---|---|---|
| history | 一整条 fish 历史命令 | `ex h` / `exh` / `Ctrl+R` |
| proctrace | 一个 pid / systemd unit / cwd（Linux only） | `ex pt` |
| ssh | 一台主机 + 一条转发规则 | `ex t` |

## 构建

```bash
cargo build --release
cargo install --path .
cargo test          # 169 tests
```

## Fish 集成

`ex <module>` 是唯一知道 exit-code 协议的地方（exit 0 插入命令行，exit 10 插入并执行），
所以新模块不需要新的 fish 函数。安装步骤见 [install/README.md](install/README.md)。

## 文档

| 想知道 | 看 |
|---|---|
| 怎么构建、怎么加模块、模块索引 | `../CLAUDE.md` |
| 定位、吸收标准、路线图 | `../docs/README.md` |
| 单个模块的设计与落地记录 | `../docs/<模块>.md` |
| 各模块的文件地图与坑 | `../.claude/rules/<模块>.md` |

## License

MIT。Copyright (c) lxb <liuxiaobo666233@gmail.com>
