#pragma once

#include "esp_err.h"
#include <stdbool.h>
#include <stdint.h>

esp_err_t passport_realtime_start(const char *gateway);
void passport_realtime_ptt_press(void);
void passport_realtime_ptt_release(void);
void passport_realtime_set_volume(uint8_t percent);
uint8_t passport_realtime_get_volume(void);
bool passport_realtime_is_ready(void);
