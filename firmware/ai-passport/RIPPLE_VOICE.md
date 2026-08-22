# Ripple Voice Passport 方案

## 产品范围

该固件把 FoloToy AI Passport 收敛成一个无登录、无历史列表的语音入口：

- 主界面只显示连接与会话状态。
- 按住 `OK` 录音，松开后立即提交。
- 回复以 24 kHz 单声道 PCM 流式播放。
- 回复播放时再次按住 `OK` 可打断并开始新一轮讲话。
- 不提供文本输入、消息列表、重播、角色选择或本地会话管理。
- 长按 `UP` 清除网络配置并重新进入配网。

## 运行架构

```text
ES8311 麦克风
  -> 16 kHz PCM16
  -> ESP32-C3 转 Float32/base64
  -> WebSocket /v1/agent/realtime（协议 v5）
  -> Ripple Agent Gateway（匿名验证账号）
  -> Responses API 编排
  -> 24 kHz Float32/base64 音频流
  -> ESP32-C3 转 PCM16
  -> ES8311 扬声器
```

设备直接连接 `140.143.229.103:8700` 的 Ripple 后端，不在设备端保存账号
token。Gateway 使用部署环境中的匿名用户配置完成快速验证。该 HTTP/WS 连接
只适用于当前产品验证阶段；TLS/WSS、一次性票据和设备身份属于后续安全加固。

## 首次配网

1. 开机后连接设备热点 `Ripple-Passport-XXXX`。
2. 浏览器打开 `http://192.168.4.1/`。
3. 填写 2.4 GHz Wi-Fi 名称和密码；Gateway 默认值无需修改。
4. 保存后设备自动重启。屏幕出现 `READY / Connected` 后即可使用。

Wi-Fi 和 Gateway 地址保存在 NVS。要更换网络，长按 `UP` 清除配置。

## 状态模型

`BOOTING -> SETUP | CONNECTING -> READY -> LISTENING -> THINKING -> SPEAKING -> READY`

网络断开后进入 `CONNECTING` 并自动重连；协议、内存或音频错误进入 `ERROR`。
UI 更新通过队列交给 LVGL 定时器处理，按键回调只投递事件；录制与播放分别在
工作任务中执行，不在 UI 或按键任务中做阻塞 I/O。

## 构建与烧录

使用 ESP-IDF 5.5.3：

```bash
source /Users/lake/esp/esp-idf-v5.5.3/export.sh
idf.py set-target esp32c3
idf.py build
idf.py -p /dev/cu.usbmodem21201 erase-flash
idf.py -p /dev/cu.usbmodem21201 flash monitor
```

分区针对实机 8 MB Flash：NVS 24 KB、PHY 4 KB、Factory App 7 MB。当前镜像约
1.46 MB。ESP32-C3 没有 PSRAM，因此录音以 40 ms 小块上传，播放队列限制为
4 块，并在打断时立即释放。

## 验收清单

- 串口启动无崩溃、看门狗或重启循环。
- ST7789 屏幕方向、边缘、颜色和背光正常。
- 配网热点与 `192.168.4.1` 页面可访问，保存后能加入目标 Wi-Fi。
- 屏幕进入 `READY`，串口出现 `websocket connected` 与 `session ready`。
- 按住 `OK` 时进入 `LISTENING`，松开进入 `THINKING`。
- 扬声器播放完整回复；播放中按住 `OK` 能立即打断。
- 连续完成至少 10 轮对话，无堆内存持续下降、音频缓冲溢出或重连循环。
