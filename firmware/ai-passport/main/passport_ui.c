#include "passport_ui.h"
#include "passport_pet.h"

#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "lvgl.h"
#include <stdio.h>

typedef struct {
    passport_ui_state_t state;
    char detail[80];
} ui_event_t;

static QueueHandle_t s_events;
static lv_obj_t *s_status;
static lv_obj_t *s_detail;
static lv_obj_t *s_orb;

static const char *state_text(passport_ui_state_t state)
{
    switch (state) {
    case PASSPORT_UI_BOOTING: return "STARTING";
    case PASSPORT_UI_SETUP: return "SETUP";
    case PASSPORT_UI_CONNECTING: return "CONNECTING";
    case PASSPORT_UI_READY: return "HOLD OK TO TALK";
    case PASSPORT_UI_LISTENING: return "LISTENING";
    case PASSPORT_UI_THINKING: return "THINKING";
    case PASSPORT_UI_SPEAKING: return "SPEAKING";
    case PASSPORT_UI_ERROR: return "OFFLINE";
    default: return "RIPPLE";
    }
}

static uint32_t state_color(passport_ui_state_t state)
{
    switch (state) {
    case PASSPORT_UI_ERROR: return 0xE45454;
    default: return 0x8B5CF6;
    }
}

static void poll_events(lv_timer_t *timer)
{
    (void)timer;
    ui_event_t event;
    while (s_events && xQueueReceive(s_events, &event, 0) == pdTRUE) {
        lv_label_set_text(s_status, state_text(event.state));
        lv_label_set_text(s_detail, event.detail);
        lv_obj_set_style_border_color(s_orb, lv_color_hex(state_color(event.state)), 0);
        passport_pet_set_state(event.state);
    }
}

void passport_ui_init(void)
{
    s_events = xQueueCreate(1, sizeof(ui_event_t));

    lv_obj_t *screen = lv_obj_create(NULL);
    lv_obj_set_style_bg_color(screen, lv_color_hex(0x08080A), 0);
    lv_obj_set_style_border_width(screen, 0, 0);
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);

    lv_obj_t *title = lv_label_create(screen);
    lv_label_set_text(title, "RIPPLE");
    lv_obj_set_style_text_font(title, &lv_font_montserrat_20, 0);
    lv_obj_set_style_text_color(title, lv_color_hex(0xF4F1E8), 0);
    lv_obj_align(title, LV_ALIGN_TOP_MID, 0, 20);

    s_orb = lv_obj_create(screen);
    lv_obj_set_size(s_orb, 174, 174);
    lv_obj_set_style_radius(s_orb, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_border_width(s_orb, 1, 0);
    lv_obj_set_style_border_color(s_orb, lv_color_hex(0x8B5CF6), 0);
    lv_obj_set_style_border_opa(s_orb, LV_OPA_50, 0);
    lv_obj_set_style_bg_color(s_orb, lv_color_hex(0x15121C), 0);
    lv_obj_align(s_orb, LV_ALIGN_CENTER, 0, -24);

    passport_pet_init(screen);

    s_status = lv_label_create(screen);
    lv_label_set_text(s_status, "STARTING");
    lv_obj_set_width(s_status, 220);
    lv_obj_set_style_text_align(s_status, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_font(s_status, &lv_font_montserrat_14, 0);
    lv_obj_set_style_text_color(s_status, lv_color_hex(0xF4F1E8), 0);
    lv_obj_align(s_status, LV_ALIGN_BOTTOM_MID, 0, -68);

    s_detail = lv_label_create(screen);
    lv_label_set_text(s_detail, "Starting hardware");
    lv_obj_set_width(s_detail, 220);
    lv_label_set_long_mode(s_detail, LV_LABEL_LONG_WRAP);
    lv_obj_set_style_text_align(s_detail, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_color(s_detail, lv_color_hex(0x8B8A91), 0);
    lv_obj_align(s_detail, LV_ALIGN_BOTTOM_MID, 0, -28);

    lv_timer_create(poll_events, 50, NULL);
    lv_screen_load(screen);
}

void passport_ui_set(passport_ui_state_t state, const char *detail)
{
    if (!s_events) return;
    ui_event_t event = {.state = state};
    snprintf(event.detail, sizeof(event.detail), "%s", detail ? detail : "");
    xQueueOverwrite(s_events, &event);
}
