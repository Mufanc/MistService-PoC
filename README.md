# MistService-PoC

> 利用 `ServiceManager::listServices` 的 `dumpPriority` 参数隐藏自定义 binder 服务，使三方 APP 无法通过枚举发现。

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

具体实现：inline hook `ServiceManager::listServices`，检测到参数中包含特殊标志位时，通过 `IPCThreadState::getCallingUid()` 检查调用者权限。如果是三方 APP 来 list，主动将标志位清零，从而使隐藏服务不出现在返回列表中。

## 难点

**注入方式**：`servicemanager` 属于 core class，在 `init.rc` 的 boot 阶段就启动了，远远早于 `post-fs-data`，因此无法使用 Magisk v25.2 那样的 bind-mount + `LD_PRELOAD` 方式注入，只能在启动后通过 ptrace 注入。

**Hook 目标**：`ServiceManager::listServices` 返回的是 `binder::Status` 结构体（16 bytes），在 AArch64 上可能通过 `q*` 寄存器或 `x8` 寄存器返回，行为不确定，导致 inline hook 的 proxy 函数难以直接定义。为此，我在 [wisp](https://github.com/Mufanc/wisp) 中实现了 intercept 功能 —— 在函数头部插入一段「拦截器」，通过参数数组修改入参，不影响原函数后续的执行逻辑。

## 工程结构

本项目包含两个子工程：

```
.
├── mist/            # 服务隐藏模块（Rust, Magisk 模块）
│   ├── src/
│   │   ├── main.rs        # 注入器入口，ptrace 注入 servicemanager
│   │   ├── lib.rs         # 动态库入口（被注入后执行）
│   │   ├── hook.rs        # hook 逻辑，拦截 listServices 并过滤隐藏服务
│   │   ├── inject.rs      # ptrace 注入实现
│   │   ├── constants.rs   # 常量定义（隐藏标志位等）
│   │   └── properties.rs  # Android 系统属性读取
│   ├── module/            # Magisk 模块模板
│   └── justfile           # 构建脚本
├── service/         # 测试用服务（Kotlin, Android 工程）
│   ├── app/               # 测试 APP，注册/列出服务
│   └── hiddenapi/         # Hidden API stubs
└── assets/          # 预编译的二进制（servicemanager、libbinder.so 等）
```

## 构建

### 前置依赖

- Android NDK（设置 `ANDROID_NDK` 环境变量）
- Rust toolchain（需要 `aarch64-linux-android` target）
- [just](https://github.com/casey/just) 命令行工具
- Android SDK（用于构建 service 工程）

### mist 模块

```bash
cd mist
just package-release
# 产物: mist/target/module.zip
```

将 `module.zip` 通过 Magisk/KernelSU 刷入即可。

### service 测试工程

```bash
cd service
./gradlew assembleDebug
# 产物: app/build/outputs/apk/debug/app-debug.ash
```

## 使用

### 1. 刷入 mist 模块

将构建好的 `module.zip` 刷入后重启，模块会在 `post-fs-data` 阶段通过 ptrace 将 hook 库注入到 `servicemanager` 进程。

### 2. 注册测试服务

将 `.ash` 文件推送到设备并执行（不带参数），会启动两个测试服务：
- `mist_service_1`：普通服务，不带隐藏 flag
- `mist_service_2`：隐藏服务，带上 `DUMP_FLAG_PRIORITY_HIDE`

```bash
adb push app-debug.ash /data/local/tmp/mist.sh
adb shell su -c /data/local/tmp/mist.sh
```

### 3. 验证隐藏效果

带 `list` 参数执行，会以特殊 flag 去 list servicemanager：

```bash
# root 权限 —— 可以看到 mist_service_2
adb shell su -c "/data/local/tmp/mist.sh list" | grep mist

# shell 权限 —— mist_service_2 应当不可见
adb shell "/data/local/tmp/mist.sh list" | grep mist
```

对比两者的输出，如果 mist 模块工作正常，shell 身份下应该看不到 `mist_service_2`。

## 说明

- 目前仅对 UID 0（root）放行隐藏服务的列出权限，这只是为了方便对比验证。实际使用中可以考虑放行 UID < 10000（即系统进程），或者根据具体需求自定义权限检查策略。
- 这是一个 PoC（概念验证），仅在 aarch64 架构上测试。
- hook 依赖 [wisp](https://github.com/Mufanc/wisp) 和 [r3solvr](https://github.com/Mufanc/r3solvr) 两个库。
