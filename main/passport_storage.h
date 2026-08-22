#pragma once

#include "esp_err.h"

#include <stddef.h>
#include <stdint.h>

typedef struct {
    char ssid[33];
    char password[65];
    char gateway[96];
} passport_wifi_config_t;

esp_err_t passport_storage_init(void);
uint8_t passport_storage_load_volume(uint8_t fallback);
esp_err_t passport_storage_save_volume(uint8_t volume);
esp_err_t passport_storage_load_wifi(passport_wifi_config_t *config);
esp_err_t passport_storage_save_wifi(const passport_wifi_config_t *config);
esp_err_t passport_storage_clear_wifi(void);
