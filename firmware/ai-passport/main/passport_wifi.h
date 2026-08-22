#pragma once

#include "esp_err.h"
#include <stddef.h>

esp_err_t passport_wifi_start(char *gateway, size_t gateway_size);
void passport_wifi_clear_config(void);
