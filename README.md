# MistService-PoC

> 注册 binder 服务到 ServiceManager 时，提供特殊 `dumpPriority` 值来实现隐藏，使三方 APP 无法发现该服务。

## 背景

许多模块需要通过劫持 `system_server` 中的服务来中转 binder 通信。然而，`servicemanager` 本身就是一座天然的桥梁 —— 为什么不直接拿来用呢？

因为存在一些问题：

`ServiceManager::checkService` 和 `ServiceManager::getService` 都会检查 SELinux 权限，未授权的进程无法访问非自身域的服务：

```
SELinux: avc:  denied  { find } for pid=3422 uid=2000 name=vold
    scontext=u:r:shell:s0 tcontext=u:object_r:vold_service:s0
    tclass=service_manager permissive=0
```

但 `ServiceManager::listServices` 不会检查权限，[所有应用都能列出全部服务](https://cs.android.com/android/platform/superproject/+/android-latest-release:frameworks/native/cmds/servicemanager/ServiceManager.cpp;l=608-634;drc=bc744ae5faf84826a1786cf65ddb53d8d9e7a4ac)，这就产生了检测点 —— 三方 APP 可以轻松发现我们注册的自定义服务。

## 原理

### 服务隐藏

注意到 `listServices` 有一个 `dumpPriority` 参数，这是在注册服务时可以指定的 bitflags。目前 AOSP 只使用了低几位，那么我们完全可以选取一个未使用的高位（比如 `1 << 24`）作为特殊标记。注册服务时带上这个 flag，就成了「隐藏服务」。

```cpp
Status ServiceManager::listServices(int32_t dumpPriority,
                                    std::vector<std::string>* outList) {
    // ...
    for (auto const& [name, service] : mNameToService) {
        if (service.dumpPriority & dumpPriority) {
            outList->push_back(name);  // 只返回 flag 匹配的服务
        }
    }
    return Status::ok();
}
```

具体实现：inline hook `ServiceManager::listServices`，检测到参数中包含特殊标志位时，通过 `IPCThreadState::getCallingUid()` 检查调用者权限。如果是未授权的进程来 list，主动将标志位清零，从而使隐藏服务不出现在返回列表中。

### 服务访问控制

除了隐藏服务外，我们还 hook 了 `Access::actionAllowedFromLookup`，这个函数负责对传入连接进行鉴权。由于三方 app 没权限 find `u:object_r:default_android_service:s0`，servicemanager 会在此处拒绝访问。通过拦截这个函数，我们可以对 `mist/` 前缀的服务放行，使三方 app 能够正常找到并访问这些服务。

### idmap 管理

为了灵活控制哪些应用可以访问隐藏服务，我们实现了一个 idmap（ID mapping）机制：

- 使用内存映射文件存储访问权限 bitmap
- 支持 UID 范围：10000-19999（普通应用）
- 通过 Binder 服务 `IMistService` 提供管理接口
- 系统进程（UID < 10000）默认拥有访问权限

## 难点

**注入方式**：`servicemanager` 属于 core class，在 `init.rc` 的 boot 阶段就启动了，远远早于 `post-fs-data`，因此无法使用 Magisk v25.2 那样的 bind-mount + `LD_PRELOAD` 方式注入，只能在启动后通过 ptrace 注入。

**Hook 目标**：`ServiceManager::listServices` 返回的是 `binder::Status` 结构体（16 bytes），在 AArch64 上可能通过 `q*` 寄存器或 `x8` 寄存器返回，行为不确定，导致 inline hook 的 proxy 函数难以直接定义。为此，我在 [wisp](https://github.com/Mufanc/wisp) 中实现了 intercept 功能 —— 在函数头部插入一段「拦截器」，通过参数数组修改入参，不影响原函数后续的执行逻辑。

**C++ 字符串处理**：hook `Access::actionAllowedFromLookup` 需要解析 C++ `std::string` 类型的服务名。由于 libc++ 的 string 实现有多种布局（long/short string，alternate layout），我们通过运行时检测自动适配不同的内存布局。

## 工程结构

本项目是一个 Cargo Workspace，包含以下 crate：

```
.
├── Cargo.toml       # Workspace 配置
├── Cargo.lock
├── justfile         # 构建脚本
├── common/          # 共享库（binder 工具、常量）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── binder.rs      # ServiceManager 多版本兼容
│       └── constants.rs   # 常量定义（隐藏标志位等）
├── daemon/          # 注入器二进制
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/
│       ├── main.rs        # CLI 入口
│       ├── daemon.rs      # Binder 服务与 idmap 管理
│       ├── inject.rs      # ptrace 注入实现
│       ├── ptrace.rs      # ptrace 操作封装
│       ├── resolver.rs    # 符号解析
│       ├── selinux.rs     # SELinux 上下文操作
│       ├── ext.rs         # 扩展函数
│       └── properties.rs  # Android 系统属性读取
├── trojan/          # 被注入的 hook 库
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs         # 动态库入口
│       ├── hook.rs        # hook 逻辑（核心）
│       └── cxx_string.rs  # C++ 字符串支持
├── service/         # 测试用服务（纯 Rust）
│   ├── Cargo.toml
│   └── src/
│       └── main.rs        # mist-poc 测试工具
├── module/          # Magisk 模块模板
│   ├── module.prop
│   ├── post-fs-data.sh    # 注入启动脚本
│   ├── customize.sh
│   ├── sepolicy.rule
│   └── bin/
└── target/          # 构建产物
```

### Workspace 结构

Rust 项目采用 Cargo Workspace 组织，包含四个 crate：

- **common**：共享库，提供多版本 ServiceManager 兼容接口和常量定义
- **daemon**：可执行二进制（`mist`），负责 ptrace 注入 servicemanager，并作为守护进程运行 Binder 服务
- **trojan**：动态库（`libmist.so`），被注入到 servicemanager 进程后执行 hook 逻辑
- **service**：可执行二进制（`mist-poc`），测试用工具，用于注册/列出服务验证功能

## 构建

### 前置依赖

- Android NDK（设置 `ANDROID_NDK` 环境变量）
- Rust toolchain（需要 `aarch64-linux-android` target）
- [just](https://github.com/casey/just) 命令行工具

### 添加 Rust target

```bash
rustup target add aarch64-linux-android
```

### 构建模块

```bash
# 构建 debug 版本
just build-debug

# 构建 release 版本
just build-release

# 打包 Magisk 模块（debug）
just package-debug

# 打包 Magisk 模块（release）
just package-release
```

将 `target/module.zip` 通过 Magisk/KernelSU 刷入即可。

### 快速测试（开发模式）

```bash
# 直接注入测试（无需刷模块）
just lite-inject
```

## 使用

### 1. 刷入模块

将构建好的 `module.zip` 刷入后重启，模块会在 `post-fs-data` 阶段通过 ptrace 将 hook 库注入到 `servicemanager` 进程，并启动 `mist` Binder 服务。

### 2. 注册测试服务

使用 `mist-poc` 工具注册测试服务：

```bash
# 推送到设备
adb push target/aarch64-linux-android/debug/mist-poc /data/local/tmp

# 启动测试服务
adb shell su -c /data/local/tmp/mist-poc service
```

这会启动两个测试服务：
- `mist/sample_visible`：普通服务，不带隐藏 flag
- `mist/sample_hidden`：隐藏服务，带上 `DUMP_FLAG_PRIORITY_HIDE`

### 3. 验证隐藏效果

使用 `list` 命令列出所有服务：

```bash
# root 权限 —— 可以看到隐藏服务
adb shell su -c "/data/local/tmp/mist-poc list" | grep mist

# shell 权限 —— 隐藏服务应当不可见
adb shell "/data/local/tmp/mist-poc list" | grep mist
```

对比两者的输出，如果模块工作正常，shell 身份下应该看不到 `mist/sample_hidden`。

### 4. idmap 管理

模块注入后，可以通过 `mist` 命令行工具管理 idmap：

```bash
# 查看当前允许的 UID 列表
adb shell su -c "/data/adb/modules/mist/bin/mist idmap list"

# 检查特定 UID 是否有权限
adb shell su -c "/data/adb/modules/mist/bin/mist idmap get 10000"

# 授予 UID 访问权限
adb shell su -c "/data/adb/modules/mist/bin/mist idmap set 10000 true"

# 撤销 UID 访问权限
adb shell su -c "/data/adb/modules/mist/bin/mist idmap set 10000 false"

# 清空所有 idmap 条目
adb shell su -c "/data/adb/modules/mist/bin/mist idmap clear"
```

**注意**：idmap 仅对 UID 范围 10000-19999（普通应用）有效。系统进程（UID < 10000）默认拥有访问权限，UID >= 20000 默认无权限。

## 说明

- 这是一个 PoC（概念验证），仅在 aarch64 架构上测试。
- hook 依赖 [wisp](https://github.com/Mufanc/wisp) 和 [r3solvr](https://github.com/Mufanc/r3solvr) 两个库。
- `IMistService` Binder 接口通过 rsbinder-aidl 在构建时自动生成。
