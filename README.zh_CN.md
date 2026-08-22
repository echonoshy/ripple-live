# Ripple AI Passport 固件

[English](README.md) | 简体中文

这是一个独立的 ESP-IDF 固件项目，把 FoloToy AI Passport 变成 Ripple Agent
Gateway 的极简按键语音终端：按住 `OK` 说话，松开后提交，然后直接收听流式回复。
设备没有登录页、聊天记录、重播、角色选择或本地会话管理。

本分支只包含设备固件。Ripple Live 移动端、Gateway 源码、部署栈和工具不属于
这里；运行时只把兼容的 Ripple Agent Gateway 当作外部服务依赖。

## 已实现功能

- 麦克风以 16 kHz 单声道采集并通过 WebSocket 流式上传。
- 回复以 24 kHz 单声道播放，使用约 400 ms 抗抖动缓冲。
- 播放中按下 `OK` 可取消当前回复并立即开始新一轮录音。
- 使用 Ripple 原始精灵动画显示连接、聆听、思考、待机、配网和异常状态。
- 使用浏览器完成 2.4 GHz Wi-Fi 与 Gateway 地址配置。
- 音量掉电保存；可临时查看电量、Wi-Fi 信号和后端连接状态。

### 按键说明

| 操作 | 功能 |
| --- | --- |
| 按住 `OK` | 开始录音 |
| 松开 `OK` | 提交本轮语音 |
| 回复播放时按下 `OK` | 打断回复并开始说话 |
| 单击 `UP` / `DOWN` | 音量增加 / 降低 10% |
| 长按 `DOWN` 1.5 秒 | 显示电量、Wi-Fi RSSI 和 AI 连接状态三秒 |
| 长按 `UP` 3 秒 | 清除 Wi-Fi 配置并重新进入配网 |

双击目前故意留空，避免增加学习成本和误操作。

## 环境要求

- FoloToy AI Passport：ESP32-C3、8 MB Flash、ST7789P3 屏幕、ES8311
  codec 和原机三键 ADC 分压电路。
- ESP-IDF 5.5.x；已验证版本为 5.5.3。
- 兼容的 Ripple Agent Gateway：提供 `/v1/agent/realtime` WebSocket v5
  协议，并在服务端配置匿名设备账号。
- 2.4 GHz Wi-Fi。ESP32-C3 不能连接仅开放 5 GHz 的网络。

## 构建与烧录

```bash
source /path/to/esp-idf-v5.5.3/export.sh
idf.py set-target esp32c3
idf.py build
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

日常升级只执行 `flash`，这样不会清除 NVS 中的 Wi-Fi 和音量。只有明确需要恢复
出厂状态时才执行 `idf.py erase-flash`。

当前固件约 2.18 MB，Factory App 分区为 7 MB。ESP32-C3 没有 PSRAM，调整图片、
任务栈、LVGL buffer 或音频队列后必须重新检查内部 RAM。

## 首次使用

1. 打开 Passport。
2. 用手机或电脑连接热点 `Ripple-Passport-XXXX`。
3. 浏览器打开 `http://192.168.4.1/`。
4. 填写 2.4 GHz Wi-Fi 名称、密码和 Gateway 主机端口。
5. 保存后设备自动重启；屏幕显示 `HOLD OK TO TALK` 即可开始对话。

验证环境默认 Gateway 是 `140.143.229.103:8700`。当前固件使用明文 `ws://`，
只适合可信验证网络；TLS/WSS 和设备身份属于量产前的独立安全加固项。

## 项目结构

```text
assets/pet-gifs/       精灵动画的固定源文件
components/bsp/        屏幕、按键、codec、电池和共享 I2C BSP
docs/                  使用、协议与硬件开发文档
main/                  产品 UI、控制、配网、实时语音和精灵运行时
main/pet_assets/       已生成的 LVGL I8 动画帧
tools/                 可重复执行的素材转换脚本
partitions.csv         8 MB Flash 分区表
sdkconfig.defaults     ESP32-C3 / LVGL 默认配置
```

## 进一步文档

- [完整使用与故障排查](docs/USER_GUIDE.zh_CN.md)
- [实时协议约定](docs/PROTOCOL.md)
- [硬件与开发说明](docs/DEVELOPMENT.md)

## 重新生成精灵资源

仓库已经提交可直接编译的 C 帧；只有固定 GIF 被有意更新时才需要重新生成：

```bash
python -m pip install pillow pypng lz4
# 同时确保 PATH 中存在 pngquant
python tools/generate_pet_assets.py
```

脚本校验原图尺寸、帧数和逐帧时长，只把完整 384×416 画布等比缩放为 144×156，
不会裁切、重绘或改变角色外观。

## 最低验收清单

- 启动进入 `session ready`，没有 watchdog、崩溃或重启循环。
- 精灵完整无裁切，各状态动画顺序和节奏正确。
- PTT 能录音、提交、播放完整回复，并能打断播放。
- 普通回复日志出现 `playback started with 400 ms buffered`，稳定网络下不反复断粮。
- 音量调整后重启仍然保留。
- 状态快捷键显示合理的电量和 RSSI。
- 连续十轮对话后 minimum free heap 没有持续下降。

更完整的配网、操作、升级和故障处理见使用指南。
