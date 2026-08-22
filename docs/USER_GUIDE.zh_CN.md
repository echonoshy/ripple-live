# Ripple AI Passport 使用指南

## 1. 设备用途

Passport 是一个极简的按键语音终端。它只负责采集语音、显示状态和播放 Ripple
Agent Gateway 返回的音频；账号、模型、上下文和 Responses API 编排全部在外部
Gateway 中完成。

典型流程：

```text
READY -> 按住 OK -> LISTENING -> 松开 OK -> THINKING
      -> SPEAKING -> READY
```

回复播放时再次按下 `OK`，设备会取消当前回复、清空本地播放队列并进入下一轮录音。

## 2. 首次配网

设备没有可用的 Wi-Fi 配置时会显示 `SETUP`，并创建开放热点：

```text
Ripple-Passport-XXXX
```

操作步骤：

1. 用手机或电脑连接该热点。
2. 打开 `http://192.168.4.1/`。
3. 输入 2.4 GHz Wi-Fi 名称和密码。
4. 输入 Gateway 地址，格式为 `主机:端口`，不要添加 `ws://` 或路径。
5. 点击保存，等待设备自动重启。

验证环境默认值是 `140.143.229.103:8700`。ESP32-C3 只支持 2.4 GHz Wi-Fi：
双频路由器可以使用，但必须开放 2.4 GHz；仅 5 GHz 的 SSID 无法连接。

成功后屏幕依次显示：

```text
CONNECTING -> HOLD OK TO TALK / Connected
```

## 3. 日常操作

### 语音对话

1. 看到 `HOLD OK TO TALK` 后按住 `OK`。
2. 屏幕进入 `LISTENING` 后自然说话。
3. 说完松开 `OK`；设备进入 `THINKING`。
4. 回复开始后屏幕显示 `SPEAKING`。

录音采用 16 kHz PCM，回复采用 24 kHz PCM。播放默认先积累约 400 ms 音频，
因此开始时间会比收到第一块数据稍晚，但能降低网络抖动造成的卡顿。已经完整到达
的短回复不会为了凑满 400 ms 而继续等待。

### 音量

- 单击 `UP`：增加 10%。
- 单击 `DOWN`：降低 10%。
- 范围是 0%–100%。
- 屏幕显示约 1.2 秒的 `VOLUME` 提示。
- 音量保存在 NVS 中，普通烧录和重启后仍然有效。

### 查看设备状态

长按 `DOWN` 约 1.5 秒，屏幕显示三秒：

```text
DEVICE STATUS
BAT 82%  4060mV
WIFI -39dBm  AI READY
```

- `BAT N/A`：电量计不存在、未初始化或本次读取失败。
- `WIFI OFFLINE`：没有连接到接入点。
- `AI OFFLINE`：Wi-Fi 可能正常，但 Gateway WebSocket 会话尚未 ready。

RSSI 参考：约 -30 dBm 很强，-50 至 -65 dBm 通常稳定，低于 -75 dBm 可能明显
增加音频断粮概率。

### 更换 Wi-Fi

长按 `UP` 满三秒。屏幕出现 `WIFI RESET / Clearing configuration...` 后，设备清除
Wi-Fi 和 Gateway 地址并重启到配网热点。音量保存在独立命名空间，不会被清除。

## 4. 屏幕状态

| 状态 | 含义 | 精灵动画 |
| --- | --- | --- |
| `STARTING` | 硬件初始化 | waiting |
| `SETUP` | 等待浏览器配网 | waving，播放一次 |
| `CONNECTING` | 正在连接 Wi-Fi 或 Gateway | waiting |
| `HOLD OK TO TALK` | 可以开始说话 | idle |
| `LISTENING` | 正在录音 | waiting |
| `THINKING` | 等待回复 | running |
| `SPEAKING` | 正在播放回复 | idle |
| `OFFLINE` | 协议、网络、内存或音频错误 | failed，播放一次 |

状态提示会覆盖底部文字，但不会改变后台的会话状态；提示结束后自动恢复。

## 5. 固件升级

连接 USB 数据线并确认串口：

```bash
ls /dev/cu.usbmodem*       # macOS
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

普通 `flash` 只覆盖 bootloader、分区表和应用区，不会擦除 NVS。不要在日常升级中
执行 `erase-flash`。完整恢复出厂状态才使用：

```bash
idf.py -p /dev/cu.usbmodemXXXX erase-flash
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

`erase-flash` 会清除 Wi-Fi、Gateway 和音量，且不可从设备恢复。

## 6. 常见问题

### 找不到设备热点

- 确认屏幕是否显示 `SETUP`。
- 如果设备仍记住旧网络，长按 `UP` 三秒进入重新配网。
- 重启后等待数秒再扫描 `Ripple-Passport-XXXX`。

### 连接 5 GHz Wi-Fi 失败

这是硬件限制。请在路由器上开启 2.4 GHz，或建立独立的 2.4 GHz SSID。双频同名
通常可用，但部分路由器的 band steering 会导致嵌入式设备配网困难，遇到问题时
建议临时分开 SSID。

### 一直显示 CONNECTING

依次检查：

1. 长按 `DOWN`，确认是否 `WIFI OFFLINE`。
2. 确认 Gateway 地址只包含主机与端口。
3. 从同一网络确认 Gateway 的 8700 端口可达。
4. 串口检查 `websocket connected`、`session ready` 或具体错误。

### 回复卡顿

- 查看 RSSI；建议高于 -70 dBm。
- 串口搜索 `playback underrun` 和 `Audio buffer overflow`。
- 正常长回复应显示 `playback started with 400 ms buffered`。
- 若网络稳定但仍卡顿，检查 Gateway 是否持续以 24 kHz、约 100 ms 一块发送音频。

### 按键没有反应或识别错误

三键共用 GPIO0 的 ADC 电阻分压。检查串口是否有 `按键就绪`，并参考开发文档的
电压窗口。不要把同时按两个键设计成可靠的组合操作。

### 屏幕卡住但音频仍工作

串口检查 watchdog 和 LVGL 日志。ESP32-C3 软件绘制大面积阴影成本很高，本项目
已使用细边框替代阴影；新增 UI 时不要恢复大半径软件阴影。

## 7. 真机验收

每次发布至少完成：

1. 冷启动和软重启各一次，无崩溃、watchdog 或重启循环。
2. 配网一次，并验证普通烧录保留 NVS。
3. 上下键分别调音量，重启确认值保留。
4. 长按下键，核对电量和 RSSI 合理。
5. 完成十轮 PTT，其中至少两次在播放中打断。
6. 检查没有反复 underrun、队列溢出或 WebSocket 重连循环。
7. 确认精灵完整无裁切，waiting/running/idle/failed 状态正确。
