#pragma once

typedef enum {
    PASSPORT_UI_BOOTING = 0,
    PASSPORT_UI_SETUP,
    PASSPORT_UI_CONNECTING,
    PASSPORT_UI_READY,
    PASSPORT_UI_LISTENING,
    PASSPORT_UI_THINKING,
    PASSPORT_UI_SPEAKING,
    PASSPORT_UI_ERROR,
} passport_ui_state_t;

void passport_ui_init(void);
void passport_ui_set(passport_ui_state_t state, const char *detail);
