#pragma once

#include "esp_err.h"
#include <stdbool.h>
#include <stddef.h>

esp_err_t passport_wifi_start(char *gateway, size_t gateway_size);
void passport_wifi_clear_config(void);
int passport_wifi_rssi(void);
bool passport_wifi_is_connected(void);
