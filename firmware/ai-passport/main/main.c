#include "passport_realtime.h"
#include "passport_ui.h"
#include "passport_wifi.h"

#include "bsp_audio.h"
#include "bsp_battery.h"
#include "bsp_button.h"
#include "bsp_display.h"
#include "bsp_i2c.h"
#include "esp_log.h"
#include "esp_system.h"
#include "nvs_flash.h"

static const char *TAG = "passport";

static void on_key(bsp_btn_t button, bsp_btn_ev_t event, void *user)
{
    (void)user;
    if (button == BSP_BTN_OK && event == BSP_BTN_PRESS) {
        passport_realtime_ptt_press();
    } else if (button == BSP_BTN_OK && event == BSP_BTN_RELEASE) {
        passport_realtime_ptt_release();
    } else if (button == BSP_BTN_UP && event == BSP_BTN_LONG) {
        passport_wifi_clear_config();
        esp_restart();
    }
}

void app_main(void)
{
    ESP_LOGI(TAG, "Ripple Passport starting (free heap: %lu)",
             (unsigned long)esp_get_free_heap_size());

    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    ESP_ERROR_CHECK(err);

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

    ESP_ERROR_CHECK(bsp_button_init(on_key, NULL));
    ESP_ERROR_CHECK(bsp_audio_init());
    bsp_audio_set_volume(70);
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
        ESP_LOGE(TAG, "Wi-Fi failed: %s", esp_err_to_name(err));
        passport_ui_set(PASSPORT_UI_ERROR, "Wi-Fi connection failed");
        return;
    }

    passport_ui_set(PASSPORT_UI_CONNECTING, "Connecting to Ripple");
    ESP_ERROR_CHECK(passport_realtime_start(gateway));
}
