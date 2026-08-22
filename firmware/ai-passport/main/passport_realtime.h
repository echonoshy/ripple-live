#pragma once

#include "esp_err.h"

esp_err_t passport_realtime_start(const char *gateway);
void passport_realtime_ptt_press(void);
void passport_realtime_ptt_release(void);
