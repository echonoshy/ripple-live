#include "passport_realtime.h"
#include "passport_storage.h"
#include "passport_ui.h"
#include "passport_wifi.h"

#include "bsp_audio.h"
#include "bsp_battery.h"
#include "bsp_button.h"
#include "bsp_display.h"
#include "bsp_i2c.h"
#include "esp_log.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include <stdio.h>

static const char *TAG = "passport";
static QueueHandle_t s_control_actions;

typedef enum {
    ACTION_VOLUME_UP = 1,
    ACTION_VOLUME_DOWN,
    ACTION_SHOW_STATUS,
    ACTION_RESET_WIFI,
} control_action_t;

static void show_device_status(void)
{
    int battery = bsp_battery_soc();
    int millivolts = bsp_battery_mv();
    int rssi = passport_wifi_rssi();
    char detail[80];
    if (battery >= 0 && millivolts >= 0) {
        if (rssi) {
            snprintf(detail, sizeof(detail), "BAT %d%%  %dmV\nWIFI %ddBm  AI %s",
                     battery, millivolts, rssi,
                     passport_realtime_is_ready() ? "READY" : "OFFLINE");
        } else {
            snprintf(detail, sizeof(detail), "BAT %d%%  %dmV\nWIFI OFFLINE  AI %s",
                     battery, millivolts,
                     passport_realtime_is_ready() ? "READY" : "OFFLINE");
        }
    } else {
        if (rssi) {
            snprintf(detail, sizeof(detail), "BAT N/A\nWIFI %ddBm  AI %s", rssi,
                     passport_realtime_is_ready() ? "READY" : "OFFLINE");
        } else {
            snprintf(detail, sizeof(detail), "BAT N/A\nWIFI OFFLINE  AI %s",
                     passport_realtime_is_ready() ? "READY" : "OFFLINE");
        }
    }
    passport_ui_notice("DEVICE STATUS", detail, 3000);
    ESP_LOGI(TAG, "status: battery=%d%% voltage=%dmV rssi=%ddBm ai=%s",
             battery, millivolts, rssi,
             passport_realtime_is_ready() ? "ready" : "offline");
}

static void control_task(void *arg)
{
    (void)arg;
    control_action_t action;
    for (;;) {
        if (xQueueReceive(s_control_actions, &action, portMAX_DELAY) != pdTRUE) continue;
        if (action == ACTION_VOLUME_UP || action == ACTION_VOLUME_DOWN) {
            int volume = passport_realtime_get_volume();
            volume += action == ACTION_VOLUME_UP ? 10 : -10;
            if (volume < 0) volume = 0;
            if (volume > 100) volume = 100;
            passport_realtime_set_volume((uint8_t)volume);
            esp_err_t err = passport_storage_save_volume((uint8_t)volume);
            if (err != ESP_OK) {
                ESP_LOGW(TAG, "volume persistence failed: %s", esp_err_to_name(err));
            }
            char detail[16];
            snprintf(detail, sizeof(detail), "%d%%", volume);
            passport_ui_notice("VOLUME", detail, 1200);
            ESP_LOGI(TAG, "volume set to %d%%", volume);
        } else if (action == ACTION_SHOW_STATUS) {
            show_device_status();
        } else if (action == ACTION_RESET_WIFI) {
            passport_ui_notice("WIFI RESET", "Clearing configuration...", 1500);
            vTaskDelay(pdMS_TO_TICKS(1200));
            passport_wifi_clear_config();
            esp_restart();
        }
    }
}

static void on_key(bsp_btn_t button, bsp_btn_ev_t event, void *user)
{
    (void)user;
    if (button == BSP_BTN_OK && event == BSP_BTN_PRESS) {
        passport_realtime_ptt_press();
    } else if (button == BSP_BTN_OK && event == BSP_BTN_RELEASE) {
        passport_realtime_ptt_release();
    } else if (button == BSP_BTN_UP && event == BSP_BTN_CLICK) {
        control_action_t action = ACTION_VOLUME_UP;
        xQueueSend(s_control_actions, &action, 0);
    } else if (button == BSP_BTN_DOWN && event == BSP_BTN_CLICK) {
        control_action_t action = ACTION_VOLUME_DOWN;
        xQueueSend(s_control_actions, &action, 0);
    } else if (button == BSP_BTN_UP && event == BSP_BTN_LONG) {
        control_action_t action = ACTION_RESET_WIFI;
        xQueueSend(s_control_actions, &action, 0);
    } else if (button == BSP_BTN_DOWN && event == BSP_BTN_LONG) {
        control_action_t action = ACTION_SHOW_STATUS;
        xQueueSend(s_control_actions, &action, 0);
    }
}

void app_main(void)
{
    ESP_LOGI(TAG, "Ripple Passport starting (free heap: %lu)",
             (unsigned long)esp_get_free_heap_size());

    esp_err_t err = passport_storage_init();
    ESP_ERROR_CHECK(err);
    passport_realtime_set_volume(passport_storage_load_volume(70));

    ESP_ERROR_CHECK(bsp_i2c_init());
    ESP_ERROR_CHECK(bsp_display_init());
    if (!bsp_lvgl_init()) {
        ESP_LOGE(TAG, "LVGL initialization failed");
        return;
    }
    bsp_display_backlight(85);

    if (bsp_lvgl_lock(1000)) {
        passport_ui_init();
        bsp_lvgl_unlock();
    }
    passport_ui_set(PASSPORT_UI_BOOTING, "Starting hardware");

    s_control_actions = xQueueCreate(8, sizeof(control_action_t));
    if (!s_control_actions ||
        xTaskCreate(control_task, "controls", 4096, NULL, 5, NULL) != pdPASS) {
        ESP_LOGE(TAG, "control task initialization failed");
        return;
    }
    ESP_ERROR_CHECK(bsp_button_init(on_key, NULL));
    ESP_ERROR_CHECK(bsp_audio_init());
    bsp_audio_set_volume(passport_realtime_get_volume());
    if (bsp_battery_init() != ESP_OK) {
        ESP_LOGW(TAG, "battery gauge unavailable");
    }
    ESP_LOGI(TAG, "hardware ready (free heap: %lu)",
             (unsigned long)esp_get_free_heap_size());

    char gateway[96] = {0};
    err = passport_wifi_start(gateway, sizeof(gateway));
    if (err == ESP_ERR_NOT_FINISHED) {
        ESP_LOGI(TAG, "waiting for browser provisioning");
        return;
    }
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Wi-Fi initialization failed: %s", esp_err_to_name(err));
        passport_ui_set(PASSPORT_UI_ERROR, "Wi-Fi initialization failed");
        return;
    }
    passport_ui_set(PASSPORT_UI_CONNECTING, "Connecting to Ripple");
    err = passport_realtime_start(gateway);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "Realtime initialization failed: %s", esp_err_to_name(err));
        passport_ui_set(PASSPORT_UI_ERROR, "Realtime initialization failed");
    }
}
