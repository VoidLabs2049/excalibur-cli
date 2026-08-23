# Excalibur 规划与设计

> 记录 Excalibur 的定位、吸收原则与各模块设计。
> 取代旧的流水账式 worklog(旧内容保留在 git 历史中,需要时 `git log` 可找回)。
>
> **2026-08-23 大修**:基于 history 的定量复核推翻了原定位和原 SSH 设计,见 §1、§5。

---

## 1. 定位

**Excalibur = 参数选择器。**

凡是「命令我会写,但那个**参数**我记不住 / shell 补全不了」的场合,给一个键盘驱动的
选择器,选完把**完整命令吐回 shell**(`ModuleAction::Output` / `OutputAndExecute`
+ fish 的 `ex`)。

这个定位取代了旧表述「把命令吐回 shell 的 shell 集成」。区别很重要:

- **吐命令回 shell 是机制,不是价值。** navi 也吐命令、fish abbrev 也吐命令。
- **价值在「补上 shell 补不了的那段参数」。** 这才是没人做的部分。

现有模块在这个视角下是同一件事,只是选的参数不同:

| 模块 | 选的参数 |
|---|---|
| history | 一整条命令(**最弱的一种**,见 §2) |
| proctrace | 一个 pid / systemd unit / cwd |
| settings | 一个 profile(自己落地,无命令可吐) |
| ssh(计划) | 一台主机 + 一条**远端路径** |

### 吸收标准

**摩擦 × 有没有现成好工具,而不是频率。** 高频但低摩擦、且已有好工具的 → 不吸收。

已判定出局:

- ❌ **git**:频率之王,但有 abbrev + 补全,每次几乎零摩擦,picker 赢不了肌肉记忆。
- ❌ **dev-utils(hash/base64/uuid/jwt…)**:两台机器 history 里零出现,且是
  copy-to-clipboard,用不上差异化。
- ❌ **单纯的「挑主机 + ssh 连接」**:fish 自带 ssh 补全已零摩擦,且 sshm / hop /
  fast-ssh / sshs 这个 niche 已很拥挤。

对已被成熟工具统治的领域(git 全功能客户端、文件管理、系统监控、docker/k8s),
策略是**整合 / 启动外部工具**,不自研。

---

## 2. 定量依据(2026-08-23,本地 fish history 2344 条)

原 worklog 的结论建立在「频率」上。复核发现三件事,直接改变结论。

### 2.1 fish history 是去重的 —— 所有「频率」其实是「参数变体数」

2344 条里只有 20 条重复,基本是并发 session 合并的残留。所以
`kami 上 rsync 740` 不是「跑了 740 次 rsync」,而是「写过 740 条**互不相同**的 rsync」。

本地实测,逐字重复率基本为零:

| 命令 | total | distinct |
|---|---|---|
| `cd` | 631 | 625 |
| `rsync` | 104 | **104** |
| `systemctl` | 43 | 43 |
| `rm` | 49 | 49 |

### 2.2 但抽掉参数后立刻塌缩成少数模板

104 条互不相同的 rsync,84 条落进 4 个模板:

```
rsync -av <HOST:PATH> .                          27
rsync -av lxb@<HOST:PATH> .                      21
rsync -av --checksum --progress <HOST:PATH> .    20
rsync -av .<PATH> <HOST:PATH>                    16
```

全局:2288 条 distinct 命令 → 1444 个形状(粗糙正则抽象,长命令压缩率远高于此)。

### 2.3 fish 对远端路径零补全

`__rsync_remote_target`(fish 自带 completions/rsync.fish)的实现是把你**已经敲的字
原样回显**,只为防止补全回退到本地文件 —— 它根本不 ssh 出去列目录。
而本地 `cd` 的 625 条路径互不相同、深达 80+ 字符
(`/var/lib/wonder/warehouseBishopNFS/warehousePool/parquet/…`)。

### 2.4 结论

**摩擦不在「哪条命令」,在「哪个参数」;而参数几乎全是「路径」和「主机」。**

推论:一个「回忆整条历史命令」的浏览器(= 现在的 history 模块)在优化一个
**很少发生**的场景 —— 长命令几乎从不逐字复用。history 模块的未来应是
**从历史里提取模板 + 参数槽**,而不是继续优化整条命令的搜索排序。

---

## 3. 当前模块

| 模块 | 说明 | 吐命令? |
|---|---|---|
| core | 应用框架:事件循环、模块系统、主菜单 | — |
| history | fish 历史浏览器(搜索 / 排序 / 复制) | ✅ Enter / Ctrl+O |
| proctrace | 查询驱动的进程检查器(name/PID/port,Linux only) | ✅ `l`/`r`/`x`/`c` |
| settings | Claude Code 配置档切换 + JSON 键值编辑 | ❌ 全部自己落地 |

shell 集成:`install/ex.fish` 定义通用 `ex <module>`,**独占** exit-code 协议;
`exh`(Ctrl+R)/ `excc` 是它的薄别名。新模块不需要新 fish 函数。

---

## 4. 路线图

1. ✅ **验证定位(2026-08-23 完成)** —— 见 §6
2. **SSH:远端路径浏览器**(下一个,见 §5)
3. **history → 模板化** —— 把 §2.2 的形状聚类做进 history 模块:同一模板下的历史
   命令折叠成一条,参数变成可填的槽。这是 §2.4 的直接推论。
4. **nix 工作流助手** —— kami 上 ~900 条又长又难记的 nix 咒语,几乎没有好 TUI。
5. (按需)外部工具启动器:为 git / 文件 / 监控等成熟领域提供一键跳入。

---

## 5. SSH 模块:远端路径浏览器(下一个)

### 5.1 相对原计划砍掉了什么,为什么

原 §4 设计了 5 个动作。按吸收标准(§1)逐个复核,**只有 2 个过关**:

| 动作 | 判定 | 理由 |
|---|---|---|
| 端口探测 | ✅ 保留 | `telnet host port` ×66,确实没好工具 |
| 传输(rsync) | ✅ 保留,且是核心 | 真痛点,且正是「远端路径补不了」的问题 |
| 连接(`ssh host`) | ❌ 砍 | fish 自带 ssh 补全已零摩擦,和「git 出局」同理 |
| 跑远程命令 | ❌ 砍 | 摩擦低;且真痛点是 fish/bash 引号地狱(见 `~/.claude/remote-ops.md`),一个朴素的 `ssh host "cmd"` 拼接器**会让这个坑更深** |
| 修 known_hosts | ❌ 砍 | 本地 3 次,`ssh-keygen -R host` 本身不长不难记 |

### 5.2 v1 设计

```
选主机(~/.ssh/config Host 块 + tailscale status)
  → 浏览远端目录树(ssh ls 驱动,可 fuzzy)      ← 唯一别人做不到的东西
      → t: 吐 rsync 拉取行(Output,可改)
  → p: 原生 TCP 端口探测(模块内执行,替掉 telnet)
```

**远端路径浏览器是整个设计里唯一「shell 补不了、现成工具也没有」的东西**,
也正是 740 条 rsync 变体的真正来源。它是 v1 的核心,不是附属功能。

### 5.3 关键设计点

1. **emit vs 原生分流**:传输 = `Output`(可编辑);端口探测 = 模块内执行,不吐命令。
   复用现有 exit-code 模式,零新架构。
2. **NixOS 安全**:`~/.ssh/config` 由 home-manager 生成,**只读解析**,绝不写入
   (否则 `nixos-rebuild` 会覆盖)。
3. **主机发现**:`~/.ssh/config` + `tailscale status`(若存在)。tailnet `100.x`
   主机是 sshm/hop 没有的差异点。
4. **Overlay 存储**:每主机的小元数据(常用端口、路径书签)—— 只读 ssh config 装不下。

### 5.4 模块结构(拟)

```
excalibur/src/modules/ssh/
├── mod.rs       # SshModule,实现 Module trait,路由主机列表 / 路径浏览 两个模式
├── state.rs     # 主机列表、选中、当前远端路径栈、overlay 存储、端口探测结果
├── ui.rs        # 主机列表 / 远端目录面板 / 端口状态
├── discovery.rs # 解析 ~/.ssh/config + tailscale status(只读)
├── browse.rs    # 远端目录列举(ssh ls),路径栈进入 / 返回
└── probe.rs     # 原生 TCP 端口探测
```

### 5.5 实现步骤

```
1. 脚手架 ModuleId::Ssh + 模块目录 + manager/CLI/菜单 注册
   → 验证:`ex ssh` 打开空模块
2. 主机发现:解析 ~/.ssh/config Host 块(+ tailscale status)
   → 验证:真实主机(zeus/thor/hades/sol/mani/fama + 100.x)渲染成模糊可搜列表
3. 远端路径浏览器:选中主机 → ssh ls 列目录 → 进入 / 返回 / fuzzy 过滤
   → 验证:能一路点进 kami:/var/lib/wonder/warehouse/database/twl/…
4. 传输:`t` → Output `rsync -av --checksum --progress <host>:<选中路径> ./`
   → 验证:吐回命令行且可改
5. 端口探测:`p` → 原生 TCP 探测,绿/红显示
   → 验证:对某主机一排端口状态正确
6. .claude/rules/ssh.md + 更新 CLAUDE.md 模块表
```

### 5.6 待定

- 远端 `ls` 的延迟:每层目录一次 ssh 往返可能慢。备选是复用一条 ControlMaster
  连接,或一次性 `find -maxdepth N`。**等步骤 3 实测再决定,不预先优化。**

---

## 6. 已完成:验证定位(2026-08-23)

在做新模块之前,先用**零新模块**的最小改动检验「参数选择器」这个定位站不站得住。

**做了什么**

1. `install/ex.fish` —— 通用 `ex <module>` wrapper,独占 exit-code 协议。
   此前 `exh` / `excc` 各自手抄一遍协议(且 `excc` 压根没处理 exit 0/10),
   号称的核心机制每加一个模块就要复制一次。现在新模块零 fish 工作量。
2. **proctrace 开始吐命令** —— 查询已经解析出了 pid / systemd unit / cwd,
   这些正是「记不住、补全不了」的参数,吐出来只是格式化:

   | 键 | 吐出 |
   |---|---|
   | `l` | `journalctl -u <unit> -f` |
   | `r` | `systemctl restart <unit>` |
   | `x` | `kill <pid>` |
   | `c` | `cd <cwd>` |

   全部用 `Output`(可编辑)而非 `OutputAndExecute` —— 这些命令会杀/重启东西。
   history 里 `systemctl` 43 + `journalctl` 21 条,直接命中。

**结论**:定位成立。proctrace 从「只读 dashboard」变成参数选择器只花了 ~40 行,
说明这个抽象是贴着现有架构长的,不是硬套。

**settings 保持不吐命令** —— 它的每个动作都是自己落地的文件操作,没有命令可交给
shell。硬造一条会是为了对称而对称。

**未验证**:emit 的端到端效果需要交互式终端,没法在此环境里跑。`cargo build` 通过、
三个 fish 文件 `fish --no-execute` 语法通过、`set -l out (cmd); set -l code $status`
的状态传递已单独实测(0 和 10 都正确)。实际按键效果需要人工确认一次。
