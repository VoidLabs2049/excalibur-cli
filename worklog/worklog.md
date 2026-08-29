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
2. **SSH:隧道仪表盘 + config 编辑器**(下一个,见 §5;原「远端路径浏览器」降级到 §5.11)
3. **history → 模板化** —— 把 §2.2 的形状聚类做进 history 模块:同一模板下的历史
   命令折叠成一条,参数变成可填的槽。这是 §2.4 的直接推论。
4. **nix 工作流助手** —— kami 上 ~900 条又长又难记的 nix 咒语,几乎没有好 TUI。
5. (按需)外部工具启动器:为 git / 文件 / 监控等成熟领域提供一键跳入。

---

## 5. SSH 模块:隧道仪表盘 + config 编辑器(下一个)

> 2026-08-29 重写。原设计(远端路径浏览器)降级为后续项,见 §5.12。

### 5.1 真实场景

在任意机器上,经跳板机开端口转发(本地 `-L` / 远程 `-R`);维护一份常用转发档案,
启动即拉起并看到连通性。外加日常的 ssh config 增改 —— **这份文件本地经常手改**,
不走 home-manager 声明式(明确否掉:失去手改敏捷性)。

### 5.2 定量依据(2026-08-29 复核本机)

| 事实 | 数据 | 推论 |
|---|---|---|
| `~/.ssh/config` 是可写普通文件 | `readlink -f` 指向自身,不是 home-manager symlink | 旧 §5.3 的「绝不写入」前提**不成立**。但仍要运行时检测(symlink 进 store / 不可写)→ 降级只读 |
| 文件很脏 | 3 种缩进(1/2/4 空格)、`Hostname` 18 处 vs `HostName` 17 处、3 处行尾空白(含 `Host xx-trade-wsl1 ` 的 pattern 自带尾空格) | 整文件重生成会产出 191 行 diff。**只替换目标 host 的行区间**即可,**不需要无损 CST** |
| **重复 `Host kami`** | 行 6 与行 60,内容相同 | OpenSSH 首次匹配生效 → 行 60 整块是死的,**且完全静默**。手改有一半概率改到死块,改完行为不变 |
| 跳板形态 100% 是 ProxyCommand | 6 处 `ProxyCommand …ssh <gw> -W %h:%p`;`ProxyJump` / `-J` 各 0 次 | 读侧必须先支持从 ProxyCommand `-W` 反解跳板,否则 6 个 host 的跳板关系不可见 |
| 监听端口绝大多数绑 loopback | 本机 `ss -tlnH` 前 12 条有 10 条是 `127.0.0.1` | **只监听 loopback 的远端端口 = 转发的头号目标**(只能经隧道访问);`0.0.0.0` 的多半直连即可。列表应据此排序高亮 |
| 转发用量看着很低 | history 里 `-L/-R/-D` 仅 5 条,`-J` 0 条 | 按**频率**过不了 §1 的标准;按「摩擦 × 有没有好工具」过得很干净 —— 痛点是**每次都要重想方向**,不是用得多 |

### 5.3 定位:为什么过 §1 的吸收标准

- **替代多步调查**:ssh 上去 → 跑 `ss -tlnH` → 肉眼挑端口 → 记住 → 退出来 →
  拼 `-L` 行 → 开个终端挂着 → 过一会儿忘了哪条还活着。
- **供给必须看见才能决定的状态**:三层连通性(§5.5)、Host 块遮蔽(§5.6)。

注意这个模块**基本不吐命令**,和 settings 同类:动作都是它自己落地。§1 的定位
(参数选择器)在这里体现为「选参数」而非「吐命令」—— 吐命令始终只是机制。

### 5.4 入口:菜单 + 预览框

沿用 settings 已验证的布局(左列表 40% + 右预览 60%)。三项按用户指定顺序,
但**默认光标停在第 3 项**(高频路径不该每次多按两下):

```
  1  修改 ssh config     预览 → host 列表摘要(alias / HostName:Port / ⤳ 跳板)
  2  修改转发 config     预览 → tunnels 档案与条目
> 3  启动 ssh 转发       预览 → 隧道状态摘要(● ● · 2 活跃 / 6)
```

第 3 项的预览框本身即满足「启动就能看到连通性」——**不进子界面就看到了**。
菜单页只跑 ①② 两层(纯本地文件读),第三个灯显示 `·`;③ 有真实 RTT,进去才跑。

### 5.5 隧道仪表盘

**连通性拆三层,分开显示才有价值:**

| 层 | 回答 | 怎么测 | 节奏 |
|---|---|---|---|
| ① 进程 | `ssh -N` 还活着吗 | procfs 扫 argv | `update()` 节流 1s |
| ② 绑定 | 本地端口真的 LISTEN 了吗 | `/proc/net/tcp` | 同上 |
| ③ 端到端 | 连过去对面真有东西吗 | `TcpStream::connect` 本地端口 | **后台线程**,进入 / `r` / 每 10s |

拆开才能看出两类静默失败:**①绿②红** = 端口被占用,转发未生效;
**①②绿③红** = 隧道通但远端服务挂了。③ 可行是因为 `-L` 是懒的 ——
本机 accept 后才建远端连接,远端失败则连接立刻被关。

**起隧道的固定形态:**

```
ssh -f -N -o BatchMode=yes -o ExitOnForwardFailure=yes -L <bind>:<target> <host>
```

- `BatchMode=yes` **不是可选的**:不加的话 agent 无 key 时 ssh 会等着读密码,
  而 TUI 正占着终端 → **界面直接卡死**。
- `ExitOnForwardFailure=yes`:不加则端口被占用时进程照常活着、转发静默失效。
- `-f` 自行 detach,excalibur **不做父进程**;每次启动扫 procfs 按 argv 精确认领。
  好处:退出 TUI 隧道继续活、重进自动看到绿灯。
- **按 pid 杀,不按模式杀** —— 天然避开 `~/.claude/remote-ops.md` 的 `pkill -f` 自匹配坑。

**`d` 远端端口发现**:`ss -tlnH` → 降级 `netstat -tln`。loopback-only 排前高亮。
选中 `Enter` 直接起、`s` 存进档案。`-R` 方向对称,扫**本地**监听端口。

### 5.6 ssh config 编辑器

**编辑全部在 TUI 内完成。** 明确否掉外部编辑器路线(`$EDITOR` / `code -g`):
excalibur 的目的之一就是不必开 VS Code,拿它当逃生舱是自相矛盾;且 `EDITOR=nano`。
反过来说这条更成立:本机 `~/.config/nvim` 为空 —— 现有编辑器**没有一个**能给
ssh_config 提供指令补全、语法校验、遮蔽检测,专用 TUI 编辑器在这个文件上确实更好。

**主路径 = 把 config 解构成结构化表单(网页表单式);直接改原文是补充。**

表单字段按**实际用量**选,不按 ssh_config 支持什么选(2026-08-29 复核 35 个块):

| 指令 | 次数 | 进表单? |
|---|---|---|
| `Port` | 35 | ✅ 数字 |
| `HostName` | 35 | ✅ 短文本 + 历史值补全 |
| `User` | 34 | ✅ 5 选 1(`lxb`/`root`/`wonder`/`deployer`/`nixos`) |
| `ProxyCommand`(跳板) | 6 | ✅ 从 35 个 alias 里选 |
| `IdentityFile` | 4 | ✅ 从 `~/.ssh` + 历史值里选 |
| `ServerAliveInterval` / `CountMax` | 3 / 3 | ❌ 落到「其他指令」 |
| `StrictHostKeyChecking` / `IdentitiesOnly` | 1 / 1 | ❌ 同上 |

**别名 + 这 5 个字段完整覆盖 35 个块里的 32 个**。例外只有 `gxzq`(5 条)、
`guosen-trade-1`(6 条)、`windows-via-reverse`(8 条),且其余指令都是「设一次不再动」
的类型。块内指令数分布:25 个块 3 条、7 个块 4 条 —— 表单是主路径,有数据撑着。

**第 1 层 · 表单**(主路径,零自由文本输入):

```
┌ 编辑 host ─────────────────────────┐
│  别名          kami                │
│  HostName      192.168.110.134     │
│  User          lxb             ▾   │
│  Port          22                  │
│  跳板          (无)            ▾   │
│  IdentityFile  (无)            ▾   │
│  ────────────────────────────────  │
│  其他指令 (0)                  ▸   │
└────────────────────────────────────┘
 j/k 切字段 · Enter 编辑 · ▾ 候选下拉 · Ctrl+S 保存 · Tab 切原文
```

**第 2 层 · 原文块编辑**(**补充**,`Tab` 切入)。编辑框里只有选中 host 那几行,
不是整个 191 行文件 —— 装得下、上下文清楚、保存时只替换该行区间。给的是现有
编辑器都没有的:

- 指令名补全(~80 个 OpenSSH 关键字,`ServerAliveCountMax` 这类没人记得住拼写)
- 值补全(`IdentityFile` 补 `~/.ssh` 下文件、`ProxyJump` 补已有 alias)
- 逐键校验,错误行当场标红
- undo/redo、词级移动、搜索(crate 的 `search` feature)
- 键位用 crate 默认(emacs 风),**不自写 vim 模态状态机**(见 §5.11)

> ⚠️ **表单存盘必须原样保留它不覆盖的指令。** `windows-via-reverse` 有 5 条表单外
> 指令;若表单按字段整块重写,这些指令会**静默消失** —— ssh 行为的变化要过很久
> 才会被发现。实现约束:表单只改它对应的那几行,其余行原样不动。

**信任机制:**

- 实时 diff 预览 —— 存盘前看到确切要写的行
- **遮蔽检测** —— 行 60 的 `Host kami` 标灰 + 「被行 6 遮蔽」
- `g` → `ssh -G <alias>` 对照 OpenSSH 实际解析出的生效值
- 原子写(临时文件 + rename)+ 自动备份 3 份 + session 内 undo + 存前语法校验

### 5.7 转发配置

存 `~/.config/excalibur/tunnels.yaml`(serde_yaml 已在依赖里,目录需新建)。
**不写进 `~/.ssh/config`** —— 那里的 `LocalForward` 语义是「每次 ssh 该 host 自动带上」,
与「我启动它才起」不同,别混。

```yaml
profiles:
  - name: 日常
    forwards:
      - host: xx-database-1
        kind: local          # local | remote
        bind: 29001
        target: 0.0.0.0:9001
        note: minio console
```

5 个字段全是选择或数字(host 从 35 个 alias 选、方向二选一、端口是数字、
target 从端口发现结果里选),第 1 层表单即可;`Tab` 切 textarea 编辑整段 yaml 备用。

### 5.8 模块结构

```
excalibur/src/modules/ssh/
├── mod.rs         # SshModule + Screen 路由(Menu / Config / Forward / Dashboard)
├── state.rs       # 各屏状态、选中、探测结果缓存
├── ui.rs          # 按 Screen 分发渲染(超 ~500 行再拆)
├── sshconfig.rs   # ~/.ssh/config 解析:alias / 起止行号 / 字段 / 跳板 / 遮蔽
├── tunnels.rs     # tunnels.yaml serde 读写
├── supervisor.rs  # 起(-f -N) / 停(按 pid) / procfs 认领
└── probe.rs       # ②绑定 ③端到端 + 后台探测线程
```

新增依赖:`tui-textarea = { version = "0.7", features = ["search"] }`
(已验证与 ratatui 0.29 + crossterm 0.28 兼容)。

CLI:`ex ssh` / 简写 `ex t`(`s` 已被 settings 占用)。

### 5.9 实现步骤

已落地(第一个 PR,110 个测试,模块内 0 警告):

```
✅ 1. 脚手架:ModuleId::Ssh + 目录 + manager/CLI 注册 + 入口菜单三项
✅ 2. sshconfig.rs 解析
      实测:35 个 host 全出、6 个跳板标对(含 ProxyCommand -W 反解)、
      kami 行 60 标出被行 6 遮蔽、round-trip 字节一致
✅ 3. 子视图 1:host 列表 + fuzzy + 6 字段表单(候选下拉)+ diff 预览
      + 原子写 + config.excalibur.bak 备份 + 保留原文件权限位
✅ 3c. g → ssh -G 生效值对照(含 Match exec 拒绝执行)
✅ 4. tunnels.rs + 子视图 2 编辑(n 新建 / Enter 改 / d 删 / Ctrl+S 存)
✅ 5. supervisor.rs:按 argv 结构认领 + 起(-f -N) / 停(按 pid)
✅ 6. probe.rs:三层灯 + 后台 worker 线程
✅ 7. 仪表盘 a 全起 / A 全停 / r 刷新 + 菜单第 3 项预览摘要
```

### 5.10 待落地

按优先级排。前三条是使用中直接提出来的,优先于原计划里剩下的。

**A. 转发界面的可读性(2026-08-29 使用反馈)**

1. **流向图形化** —— 详情面板现在是两行文字(`listen here / exit from kami`)。
   改成竖向流水图,把三段(入口 / 跳板 / 出口)画出来:

   ```
   in   here             6022
        │  through ssh
   hop  kami
        │  kami connects to
   out  localhost:22   (= kami itself)
   ```

   `-R` 时第一段换成对端。**只用 ratatui 已在用的制表符**(`│`),
   不用 `▼`/`→` 这类东亚宽度歧义字符。
2. **左栏显示 Note** —— 现在只有 `-L 6022:localhost:22 kami`,备注看不到,
   而备注往往是唯一能说清这条规则用途的东西。第二行 dim 显示。
3. **仪表盘统计** —— 每条:运行时长 + 吞吐速率;汇总:up / stopped /
   incomplete 计数与总速率;选中项画 sparkline。
   数据源 `/proc/<pid>/stat` 的 starttime 与 `/proc/<pid>/io` 的 rchar+wchar。
   ⚠️ **读之前 pid 必须已由 argv 匹配确认属于目标隧道**,否则量的是别的进程
   (见 `~/.claude/remote-ops.md`「观测之前先确认观测对象是对的」)。
   实现上把计数与身份分开:`supervisor::usage(pid) -> Usage`,不塞进 `Running`。

**B. 原计划剩余**

4. **`d` 远端端口发现** —— `ssh <host> ss -tlnH`,降级 `netstat -tln`。
   **只监听 loopback 的排前面并高亮**(本机实测 12 条里 10 条如此),
   因为那些正是只能靠 `-L` 访问的。选中 `Enter` 直接起 / `s` 存进档案。
5. **`n` 新建 / `c` 克隆 host** —— 三模板(基础 / 经跳板 / 带 IdentityFile);
   克隆追加到文件末尾,不碰现有字节。对 `xx-trade-wsl1..4` 这种只差端口的族群。
6. **3b 块内多行编辑** —— `Tab` 切 `tui-textarea` 编辑选中 host 的那几行,
   带 ~80 个 OpenSSH 关键字补全与逐键校验。
   现状:`tui-textarea` 依赖已引入,但只用在表单的单行字段上。
   **优先级最低** —— 6 字段表单已覆盖 35 个块里的 32 个,这一层是补充。
7. **文档** —— `.claude/rules/ssh.md` + CLAUDE.md 模块表 + 本节收尾。

**C. 已知边界(不是 bug,写下来免得重复发现)**

- `-R` 的第二盏灯恒为 `-`:端口开在对端,本机无法观测。能测的是出口
  (要暴露的服务在本机活着没有),已如此实现。
- `-R` 还需要远端 sshd `GatewayPorts yes` 才能被第三台机器访问,详情面板已提示。
- `-D`(SOCKS)不做,history 里 0 次使用。
- 自动重连不做,红灯 + 一键重起。
```

风险点:2(脏 config 解析 + 遮蔽)、5(argv 认领)、6(三层灯语义)。

### 5.11 明确砍掉的(以及为什么)

| 砍掉 | 理由 |
|---|---|
| 无损 CST parser | 只替换目标 host 的行区间即可,其余字节天然不动。CST 是「整文件重生成」路线才需要的 |
| `$EDITOR` / `code -g` 逃生舱 | 与「不必开 VS Code」的目的自相矛盾;且第 2 层能编辑任意内容,没有兜不住的情况 |
| `ModuleAction::Suspend` | 上条的连带 —— 不再需要挂起终端,**零 core 改动** |
| vim 模态状态机(~200 行) | 块内自由文本编辑**不是常用场景** —— 日常改 config 落在第 1 层表单(选择/数字),原文编辑只是补充。用 crate 默认键位即可;半吊子 vim 比没有 vim 更烦 |
| home-manager 声明式生成 | 能消除重复块/重复劳动整类问题,但 config 变只读、改配置要 rebuild,失去手改敏捷性 |
| tunnels 的 JSON Schema + LSP | 同样基于 VS Code,一并撤 |
| `-D` SOCKS | history 里 0 次使用 |
| 自动重连 | 引入后台循环,且「它怎么自己又起来了」很困惑。红灯 + 一键重起够用 |
| 模块内起前台隧道 / 完整生命周期管理 | `-f` detach + procfs 认领已覆盖,不必持有子进程状态 |

### 5.12 后续:远端路径浏览器(原设计,降级保留)

§2.3 的定量依据仍然成立(fish 对远端路径零补全,`__rsync_remote_target` 只回显;
104 条 rsync 全不重复)。选主机 → `ssh ls` 走目录树 → `t` 吐 rsync 拉取行。
它与本节共用 `sshconfig.rs` 的主机发现,等 §5.9 跑完再排期。

待定:远端 `ls` 的延迟(每层一次 ssh 往返)。备选复用 ControlMaster 或一次性
`find -maxdepth N`。**实测再决定,不预先优化。**

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
