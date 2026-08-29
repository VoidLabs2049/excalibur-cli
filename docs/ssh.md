# SSH 模块设计

> 隧道仪表盘 + ssh config 结构化编辑器。
> 2026-08-29 立项并完成第一版(PR #3)。本文是模块级设计文档,项目级定位见
> `docs/README.md`。

---

## 1. 场景

在任意机器上,经跳板机开端口转发(本地 `-L` / 远程 `-R`);维护一份常用转发档案,
启动即拉起并看到连通性。外加日常的 ssh config 增改 —— **这份文件本地经常手改**,
不走 home-manager 声明式(明确否掉,见 §8)。

**编辑全部在 TUI 内完成。** 明确否掉外部编辑器路线(`$EDITOR` / `code -g`):
excalibur 的目的之一就是不必开 VS Code,拿它当逃生舱是自相矛盾;且 `EDITOR=nano`,
本机 `~/.config/nvim` 为空。反过来这条更成立 —— 现有编辑器**没有一个**能给
ssh_config 提供指令补全、语法校验、遮蔽检测。

## 2. 定量依据(2026-08-29 复核本机)

| 事实 | 数据 | 推论 |
|---|---|---|
| `~/.ssh/config` 是可写普通文件 | `readlink -f` 指向自身,不是 home-manager symlink | 「绝不写入」的前提不成立。但仍要运行时检测(symlink 进 store / 不可写)→ 降级只读 |
| 文件很脏 | 3 种缩进(1/2/4 空格)、`Hostname` 18 处 vs `HostName` 17 处、3 处行尾空白(含 `Host xx-trade-wsl1 ` 的 pattern 自带尾空格) | 整文件重生成会产出 191 行 diff。**只替换目标 host 的行区间**即可,**不需要无损 CST** |
| **重复 `Host kami`** | 行 6 与行 60,内容相同 | OpenSSH 首次匹配生效 → 行 60 整块是死的,**且完全静默**。手改有一半概率改到死块,改完行为不变 |
| 跳板形态 100% 是 ProxyCommand | 6 处 `ProxyCommand …ssh <gw> -W %h:%p`;`ProxyJump` / `-J` 各 0 次 | 读侧必须先支持从 ProxyCommand `-W` 反解跳板,否则 6 个 host 的跳板关系不可见 |
| 监听端口绝大多数绑 loopback | 本机 `ss -tlnH` 前 12 条有 10 条是 `127.0.0.1` | **只监听 loopback 的远端端口 = 转发的头号目标**(只能经隧道访问);`0.0.0.0` 的多半直连即可。列表应据此排序高亮 |
| 转发用量看着很低 | history 里 `-L/-R/-D` 仅 5 条,`-J` 0 条 | 按**频率**过不了吸收标准;按「摩擦 × 有没有好工具」过得很干净 —— 痛点是**每次都要重想方向**,不是用得多 |

**为什么过吸收标准:** 替代一串多步调查(ssh 上去 → 跑 `ss -tlnH` → 肉眼挑端口 →
记住 → 退出来 → 拼 `-L` 行 → 开个终端挂着 → 过一会儿忘了哪条还活着),
并供给「必须看见才能决定」的状态:三层连通性(§4)、Host 块遮蔽(§5)。

这个模块**基本不吐命令**,和 settings 同类:动作都是它自己落地的。
「参数选择器」的定位在这里体现为**选参数**而非**吐命令** —— 吐命令始终只是机制。

## 3. 入口:菜单 + 预览框

沿用 settings 已验证的布局(左列表 40% + 右预览 60%)。三项按用户指定顺序,
但**默认光标停在第 3 项**(高频路径不该每次多按两下):

```
  1  修改 ssh config     预览 → host 列表摘要(alias / HostName:Port / 跳板)
  2  修改转发 config     预览 → tunnels 档案与条目
> 3  启动 ssh 转发       预览 → 隧道状态摘要(2 of 6 up)
```

第 3 项的预览框本身即满足「启动就能看到连通性」——**不进子界面就看到了**。
菜单页只跑第 ①② 层(纯本地文件读),第三个灯显示 `-`;③ 有真实 RTT,进去才跑。

## 4. 隧道仪表盘

**连通性拆三层,分开显示才有价值:**

| 层 | 回答 | 怎么测 | 节奏 |
|---|---|---|---|
| ① 进程 | `ssh -N` 还活着吗 | procfs 扫 argv | 1s |
| ② 绑定 | 本地端口真的 LISTEN 了吗 | `/proc/net/tcp` | 1s |
| ③ 端到端 | 连过去对面真有东西吗 | `TcpStream::connect` 本地端口 | **后台 worker**,进入 / `r` / 每 10s |

拆开才能看出两类静默失败:**①绿②红** = 端口被占用,转发未生效;
**①②绿③红** = 隧道通但远端服务挂了。③ 可行是因为 `-L` 是懒的 ——
本机 accept 后才建远端连接,远端失败则连接立刻被关。

**起隧道的固定形态:**

```
ssh -f -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ConnectTimeout=10 \
    -L <bind>:<target> <host>
```

- `BatchMode=yes` **不是可选的**:不加的话 agent 无 key 时 ssh 会等着读密码,
  而 TUI 正占着终端 → **界面直接卡死**。
- `ExitOnForwardFailure=yes`:不加则端口被占用时进程照常活着、转发静默失效。
- `ConnectTimeout=10`:不加则一台不可达的机器会占住 worker 直到 TCP 超时。
- `-f` 自行 detach,excalibur **不做父进程**;每次启动扫 procfs 按 argv 精确认领。
  好处:退出 TUI 隧道继续活、重进自动看到绿灯。
- **按 pid 杀,不按模式杀** —— 天然避开 `~/.claude/remote-ops.md` 的
  `pkill -f` 自匹配坑。

**认领是结构化的,不是字符串搜索。** `parse_argv` 按 ssh 的选项语法走一遍 argv:
`TAKES_VALUE` 里的字母吃掉下一个词,剩下第一个裸词才是目标主机。这样
`ssh -p 2222 -o X=y -L 1:2:3 realhost` 的主机是 `realhost` 不是 `2222`,
而 `ProxyCommand` 派生的 `ssh gw -W %h:%p` 与 `ssh -N -D 1080` 都不会被误认成隧道。

**一条规则 = 一个 ssh 进程。** ssh 支持一个进程挂多个 `-L`,不采用:
`ExitOnForwardFailure` 会变成一条端口被占就整组倒;`Health` 不再与 pid 一一对应,
三层灯失去落点;`parse_argv` 返回一个三元组要改成 N 个,而按 argv 结构认领正是
不能退回模式匹配的那块地基。一组多端口开多条连接的开销,用 ssh config 的
`ControlMaster` 解 —— 那是配置问题,不是结构问题。

## 5. ssh config 编辑器

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

**别名 + 这 5 个字段完整覆盖 35 个块里的 32 个。** 例外只有 `gxzq`(5 条)、
`guosen-trade-1`(6 条)、`windows-via-reverse`(8 条),且其余指令都是「设一次
不再动」的类型。块内指令数分布:25 个块 3 条、7 个块 4 条 —— 表单是主路径,
有数据撑着。

> ⚠️ **表单存盘必须原样保留它不覆盖的指令。** `windows-via-reverse` 有 5 条表单外
> 指令;若表单按字段整块重写,这些指令会**静默消失** —— ssh 行为的变化要过很久
> 才会被发现。实现约束:表单只改它对应的那几行,其余行原样不动。

**第 2 层 · 原文块编辑**(补充,未落地,见 §7)。编辑框里只有选中 host 那几行,
不是整个 191 行文件 —— 装得下、上下文清楚、保存时只替换该行区间。

**信任机制:**

- 实时 diff 预览 —— 存盘前看到确切要写的行
- **遮蔽检测** —— 行 60 的 `Host kami` 标灰 +「被行 6 遮蔽」
- `g` → `ssh -G <alias>` 对照 OpenSSH 实际解析出的生效值
- 原子写(临时文件 + rename)+ 备份 + 保留原文件权限位
  (ssh 会拒绝权限过松的 config,新建文件会静默取 umask 默认值)

## 6. 转发配置

存 `~/.config/excalibur/tunnels.yaml`。**不写进 `~/.ssh/config`** ——
那里的 `LocalForward` 语义是「每次 ssh 该 host 自动带上」,与「我启动它才起」不同,
别混。

```yaml
profiles:
  - name: daily
    forwards:
      - host: kami
        kind: local          # local (-L) | remote (-R)
        bind: '6022'
        target: localhost:22
        note: SSH_kami
```

**`Profile` 就是「转发配置组」** —— 数据模型已经有了。缺的全在 UI(见 §7 A)。

**方向的两半都会翻转,而语法把第二半藏起来了。** `-L port:host:hostport` 里
出口地址是**对端解析**的:`-L 29001:10.0.0.5:9001 kami` 是 kami 去连 10.0.0.5,
所以出口可以写任何 kami 能到、而本机到不了的地址。`-R` 完全镜像。
`localhost` 作为出口是最容易读反的一处,UI 直接标注成 `(= kami itself)` /
`(= this machine)`。

**出口必须写全 `host:port`。** 只写端口会拼出 `-L 22:6022`,ssh 直接拒绝,
而字段本身没有任何地方提示它需要两段 —— 所以 `problem()` 提前拦下并给出
`localhost:6022` 这样的具体改法,`normalise_target()` 在提交时把裸端口展开成
`localhost:<port>`,让字段显示的内容与实际命令一致。

## 7. 模块结构

```
excalibur/src/modules/ssh/
├── mod.rs         # SshModule + Screen 路由(Menu / Config / Forward / Dashboard)
├── state.rs       # 各屏状态、选中、探测结果缓存
├── ui.rs          # 按 Screen 分发渲染
├── sshconfig.rs   # ~/.ssh/config 解析:alias / 起止行号 / 字段 / 跳板 / 遮蔽
├── effective.rs   # ssh -G 生效值对照
├── form.rs        # 结构化表单 + diff plan + 原子写
├── tunnels.rs     # tunnels.yaml serde 读写 + 规则校验
├── supervisor.rs  # 起(-f -N) / 停(按 pid) / procfs 按 argv 认领
├── probe.rs       # ②绑定 ③端到端
└── worker.rs      # 阻塞工作(起/停/探测)移出渲染线程
```

依赖:`tui-textarea = { version = "0.7", features = ["search"] }`
(已验证与 ratatui 0.29 + crossterm 0.28 兼容)。

CLI:`ex ssh` / 简写 `ex t`(`s` 已被 settings 占用)。

---

## 8. 已落地(PR #3,110 个测试,模块内 0 警告)

```
✅ 脚手架:ModuleId::Ssh + manager/CLI 注册 + 入口菜单三项 + 预览摘要
✅ sshconfig.rs 解析
   实测:35 个 host 全出、6 个跳板标对(含 ProxyCommand -W 反解)、
   kami 行 60 标出被行 6 遮蔽、round-trip 字节一致
✅ 子视图 1:host 列表 + fuzzy + 6 字段表单(候选下拉)+ diff 预览
   + 原子写 + config.excalibur.bak 备份 + 保留权限位 + 只读检测
✅ g → ssh -G 生效值对照(含 Match exec 拒绝执行)
✅ tunnels.rs + 子视图 2 编辑(n 新建 / Enter 改 / d 删 / Ctrl+S 存)
   + problem() 规则校验 + 出口裸端口展开 + 方向图解
✅ supervisor.rs:按 argv 结构认领 + 起(-f -N)/ 停(按 pid)
✅ probe.rs:三层灯 + 后台 worker 线程
✅ 仪表盘 a 全起 / A 全停 / r 刷新
```

## 9. 待落地

共 11 项(A1、A2 已完成)。分组内按顺序做,A / B 来自 2026-08-29 的使用反馈,优先于其余。

### A · 转发组与多选

~~**A1 · 组要看得见**~~ —— **2026-08-29 完成**(115 个测试)。

```
 daily                                      2/3 up
   * * *  -L 6022:localhost:22 kami         pid 41233
          SSH_kami
   * * *  -L 6023:localhost:22 apollo       pid 41290
          SSH_apollo
   o o o  -L 9001:10.0.0.5:9001 kami        stopped
          minio console
```

- 组标题带 `n/m up`,颜色随状态(全起绿 / 全停灰 / 部分黄);光标所在组的标题变青,
  不移动光标也知道自己在哪组。
- 标题行**不可选中**,光标仍只走规则行,`forward_index` 语义不变。可选中的标题会
  牵动 `forward_next/previous`、`selected_forward`、`open_forward_form`、
  `delete_forward`、`selected_slot` 全部;而「对光标所在的组动手」本身没有歧义,
  不需要把标题变成一个可停靠的位置。
- **Note 作为第二行 dim 显示**,仪表盘与转发编辑页都有。缩进随屏不同 ——
  仪表盘要让开三盏灯(第 10 列),编辑页不用(第 5 列)。
- 组计数与规则状态对齐在同一列(`STATUS_COLUMN = 44`),窄栏(转发页占 40%)
  自动左移,否则会被整个裁掉。

**实现上比原计划多动了 `state.rs`**:`profile_status()` 与 `selected_profile()`
要读 `running`,渲染层拿不到。

**顺带修了一个既有隐患**:转发编辑页的左栏原本用无状态 `List`,选中项超出可视区
就会静默消失。Note 让每行高度翻倍,把这个隐患变成了现实,所以两栏都改成
`StatefulWidget` + `ListState`。标题与 Note 都占行,所以 item 下标不再等于规则
下标 —— 选中项在构建列表时记录,而不是回算。

~~**A2 · 组要能建**~~ —— **2026-08-29 完成**(121 个测试)。

原计划是加 `N` 新建组 / `R` 重命名两个键 + 一个文本输入框。**没这么做** ——
把「组」变成表单的第一个字段更省,而且顺带解决了原计划没覆盖的两件事:

```
> Group       daily                    ← Pick(已有组) / Tab 改文本 = 新建组
  Host        nowhere
  Direction   local
  Listen on   39003
  Exit at     127.0.0.1:1
  Note        minio
```

- **建组** = 在 Group 字段按 `Tab` 打一个不存在的名字。存盘时若找不到同名组就
  新建一个。于是「空组无处存在」是**结构上成立**的,不靠删除时 prune ——
  组当且仅当有规则指着它时存在。
- **换组** = 改这个字段。存盘时先从原组摘掉再放进目标组,顺序反了会留下副本。
- **`n` 选组**原计划要单独做,现在是同一个字段,零额外机制。
- `c` **克隆选中规则**,bind 端口自动跳到下一个没人占的值(两条规则同端口起不来)。
  地址前缀保留,只动端口。一组多端口时 6 个字段有 5 个重复,克隆把「再加一个端口」
  降到改 1 格 —— 这是「组里涉及多个端口」最直接的那把钥匙。

**发现的坑:`Tab` 完全无法被发现。** placeholder 只在字段为空时显示,而 Group
字段永远有默认值 —— 于是唯一的建组入口没有任何提示。补了一条 Pick 打开时的底栏:
`j/k: choose   Enter: accept   Tab: type a new one   Esc: cancel`。
(**config 编辑器那边的 Pick 有同样的问题**,是既有缺陷,没顺手改。)

**测试不能碰真实文件。** `save_forward_form` 直接写
`~/.config/excalibur/tunnels.yaml`,几个新测试差点把本机配置覆盖掉。拆成
`apply_forward_form()`(纯内存,可测)+ 写盘两步。

**未做:`R` 重命名组。** 需要一个文本输入框,当前没有别的东西要用它;而且改名可以
靠逐条改 Group 字段达成(3 条规则改 3 次),不阻塞。等真需要时再说。

**A3 · 多选启动**(`state.rs` + `mod.rs` + `ui.rs`)

组是**声明**的(存进 yaml,今后一直用),多选是**临时**的(这一次只要这两条)。
只做组 →「今天只重起其中两条」得一条条按;只做多选 → 每次开工重新勾一遍,
正是这个工具要消掉的重复劳动。两个都要。

`marked: HashSet<Slot>`,3 个选择键 + 2 个动作键,现有键位含义一个不改:

| 键 | 作用 |
|---|---|
| `Space` | 勾选 / 取消当前规则 |
| `g` | 勾选 / 取消**当前组全部** |
| `u` | 清空勾选 |
| `s` / `S` | 起 / 停**作用域** |
| `Enter` | 起停当前行(不变,忽略勾选) |
| `a` / `A` | 全起 / 全停(不变) |

**作用域 = 有勾选则勾选集,否则光标行。** ranger/lf 的老规矩,好处是常用路径零新增
按键;代价是同一个键在不同时刻做不同的事 —— **所以底栏必须实时写明作用域**
(`s: start 2 marked` / `s: start this one`),否则它就是个看不见的模态。

**集合动作只有起和停,没有 toggle。** 勾中两条一起一停时 toggle 该往哪边倒没有
答案;单条 `Enter` 才有资格 toggle。

抽出 `start_slots(&[Slot])` 给 `a` 与 `s` 共用,继承既有约定:规则不完整时报
**跳过计数**(`Starting 3, skipped 1 incomplete`),不静默少起一条。

勾选跨扫描保留,但删规则 / 重载文件时清空 —— `Slot` 是下标,文件一变就指错行。

验证:4 条勾 2 条,`s` 只起那 2 条;勾选集里混一条 incomplete,提示报出跳过数;
删掉一条规则后勾选不残留。

### B · 可读性

**B1 · 流向图形化** —— 详情面板现在是两行文字(`listen here / exit from kami`)。
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

**B2 · 仪表盘统计** —— 每条:运行时长 + 吞吐速率;汇总:up / stopped / incomplete
计数与总速率;选中项画 sparkline。数据源 `/proc/<pid>/stat` 的 starttime 与
`/proc/<pid>/io` 的 rchar+wchar。

⚠️ **读之前 pid 必须已由 argv 匹配确认属于目标隧道**,否则量的是别的进程
(见 `~/.claude/remote-ops.md`「观测之前先确认观测对象是对的」)。
实现上把计数与身份分开:`supervisor::usage(pid) -> Usage`,不塞进 `Running`。

### C · 兑现 §5「信任机制」里承诺过但没落地的

**C1 · 存盘前语法校验** —— §5 承诺了,没做。`effective.rs` 已经能拿 `ssh -G` 当
校验器,但只在按 `g` 时手动触发,**写盘前不跑**。表单写出一个 OpenSSH 拒绝解析的值
(比如 `Port` 填了非数字),整个 config **从那行起全部失效**,而且是静默的 ——
只有下次连接时才报错,那时早已不记得改过什么。
做法:把 `plan.lines` 写进临时文件,`ssh -F <temp> -G <alias> true`,非零退出则
拒绝保存并显示 stderr。

**C2 · 撤销与备份语义** —— §5 承诺了「session 内 undo」,没做。
现在 `write_config` 每次保存都覆盖同一个 `config.excalibur.bak`,所以**连存两次
就再也回不到原始文件**。备份只有一份是明确要求的(2026-08-29),没问题;
问题是「有备份」听起来比实际能力强。两条路选一:
(a) session 内保留首次进入时的原文,提供 `U` 撤销到进入前;
(b) 至少在 UI 上写明「备份 = 上一次保存前」,不让人误以为是原始文件。
倾向 (a) —— 内存里存一份 `Vec<String>` 而已,成本极低。

**C3 · 无主隧道** —— 改一条正在跑的规则(bind 6022 → 6024),旧进程还活着、
还占着 6022,但 `find()` 按 `(kind, spec, host)` 三项全等匹配,**它直接从面板上
消失了** —— 端口被占,却看不到是谁占的。`scan()` 本来就返回本机全部 ssh 转发进程,
只是没匹配上的被丢掉了。列表底部加一段「无主隧道」:显示 pid 与 spec,可停。
**便宜,可以插队。** 这正是本模块的主张 —— 静默失败要看得见。

顺带:`Tunnels::save` 只做 temp + rename,**没有备份**,与 config 侧不对称。
yaml 是本工具自己写的、结构简单,风险低于 config,但值得对齐。

### D · 原计划剩余

**D1 · `d` 远端端口发现** —— `ssh <host> ss -tlnH`,降级 `netstat -tln`。
**只监听 loopback 的排前面并高亮**(本机实测 12 条里 10 条如此),因为那些正是
只能靠 `-L` 访问的。`-R` 方向对称,扫**本地**监听端口。
与 A2/A3 合起来最划算:发现 kami 上 5 个 loopback 端口 → `Space` 勾选 →
「存成组」,一次建好。

**D2 · host `n` 新建 / `c` 克隆** —— 三模板(基础 / 经跳板 / 带 IdentityFile);
克隆追加到文件末尾,不碰现有字节。对 `xx-trade-wsl1..4` 这种只差端口的族群。

**D3 · 块内多行编辑** —— `Tab` 切 `tui-textarea` 编辑选中 host 的那几行,带
~80 个 OpenSSH 关键字补全(`ServerAliveCountMax` 这类没人记得住拼写)与逐键校验。
依赖已引入但目前只用在表单的单行字段上。
**优先级最低** —— 6 字段表单已覆盖 35 个块里的 32 个,这一层是补充。

~~**D4 · 文档**~~ —— 2026-08-29 完成。`.claude/rules/ssh.md`、CLAUDE.md 模块表与
架构树、`docs/` 索引都已补上,即 CLAUDE.md「Adding a New Module」的第 5、6 步。

### 风险点

A3 的作用域是隐式模态(靠底栏文案兜)、B2 的 pid 归属、C3 与既有认领逻辑的边界
(无主 = `scan()` 有而 `find()` 无,**不能反过来靠模式匹配补**)。

---

## 10. 明确砍掉的(以及为什么)

| 砍掉 | 理由 |
|---|---|
| 无损 CST parser | 只替换目标 host 的行区间即可,其余字节天然不动。CST 是「整文件重生成」路线才需要的 |
| `$EDITOR` / `code -g` 逃生舱 | 与「不必开 VS Code」的目的自相矛盾;且第 2 层能编辑任意内容,没有兜不住的情况 |
| `ModuleAction::Suspend` | 上条的连带 —— 不再需要挂起终端,**零 core 改动** |
| vim 模态状态机(~200 行) | 块内自由文本编辑**不是常用场景** —— 日常改 config 落在表单(选择/数字),原文编辑只是补充。用 crate 默认键位即可;半吊子 vim 比没有 vim 更烦 |
| home-manager 声明式生成 | 能消除重复块整类问题,但 config 变只读、改配置要 rebuild,失去手改敏捷性 |
| tunnels 的 JSON Schema + LSP | 同样基于 VS Code,一并撤 |
| 一个 ssh 进程挂多个 `-L` | 见 §4:毁掉 `ExitOnForwardFailure`、三层灯与 argv 认领三样。连接数用 `ControlMaster` 解 |
| `-D` SOCKS | history 里 0 次使用 |
| 自动重连 | 引入后台循环,且「它怎么自己又起来了」很困惑。红灯 + 一键重起够用 |
| 模块内起前台隧道 / 完整生命周期管理 | `-f` detach + procfs 认领已覆盖,不必持有子进程状态 |

## 11. 已知边界(不是 bug,写下来免得重复发现)

- **`-R` 的第二盏灯恒为 `-`**:端口开在对端,本机无法观测。能测的是出口
  (要暴露的服务在本机活着没有),已如此实现。
- `-R` 还需要远端 sshd `GatewayPorts yes` 才能被第三台机器访问,探测详情已提示。
- 三层灯的 ② 只按端口匹配 `/proc/net/tcp`,不区分绑定地址。
- 非 Linux 平台 `supervisor::scan()` 返回空,仪表盘全显示 stopped 而不是构建失败。
