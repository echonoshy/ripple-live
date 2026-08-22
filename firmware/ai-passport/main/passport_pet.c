#include "passport_pet.h"

#define DECLARE_FRAME(state, index) LV_IMAGE_DECLARE(passport_pet_##state##_##index)
#define ARRAY_COUNT(items) ((uint8_t)(sizeof(items) / sizeof((items)[0])))

DECLARE_FRAME(idle, 0); DECLARE_FRAME(idle, 1); DECLARE_FRAME(idle, 2);
DECLARE_FRAME(idle, 3); DECLARE_FRAME(idle, 4); DECLARE_FRAME(idle, 5);
DECLARE_FRAME(waiting, 0); DECLARE_FRAME(waiting, 1); DECLARE_FRAME(waiting, 2);
DECLARE_FRAME(waiting, 3); DECLARE_FRAME(waiting, 4); DECLARE_FRAME(waiting, 5);
DECLARE_FRAME(running, 0); DECLARE_FRAME(running, 1); DECLARE_FRAME(running, 2);
DECLARE_FRAME(running, 3); DECLARE_FRAME(running, 4); DECLARE_FRAME(running, 5);
DECLARE_FRAME(waving, 0); DECLARE_FRAME(waving, 1); DECLARE_FRAME(waving, 2);
DECLARE_FRAME(waving, 3);
DECLARE_FRAME(failed, 0); DECLARE_FRAME(failed, 1); DECLARE_FRAME(failed, 2);
DECLARE_FRAME(failed, 3); DECLARE_FRAME(failed, 4); DECLARE_FRAME(failed, 5);
DECLARE_FRAME(failed, 6); DECLARE_FRAME(failed, 7);

typedef struct {
    const lv_image_dsc_t *const *frames;
    const uint16_t *durations_ms;
    uint8_t count;
    bool loop;
} pet_animation_t;

static const lv_image_dsc_t *const IDLE_FRAMES[] = {
    &passport_pet_idle_0, &passport_pet_idle_1, &passport_pet_idle_2,
    &passport_pet_idle_3, &passport_pet_idle_4, &passport_pet_idle_5,
};
static const uint16_t IDLE_DURATIONS[] = {280, 110, 110, 140, 140, 320};

static const lv_image_dsc_t *const WAITING_FRAMES[] = {
    &passport_pet_waiting_0, &passport_pet_waiting_1, &passport_pet_waiting_2,
    &passport_pet_waiting_3, &passport_pet_waiting_4, &passport_pet_waiting_5,
};
static const uint16_t WAITING_DURATIONS[] = {150, 150, 150, 150, 150, 260};

static const lv_image_dsc_t *const RUNNING_FRAMES[] = {
    &passport_pet_running_0, &passport_pet_running_1, &passport_pet_running_2,
    &passport_pet_running_3, &passport_pet_running_4, &passport_pet_running_5,
};
static const uint16_t RUNNING_DURATIONS[] = {120, 120, 120, 120, 120, 220};

static const lv_image_dsc_t *const WAVING_FRAMES[] = {
    &passport_pet_waving_0, &passport_pet_waving_1,
    &passport_pet_waving_2, &passport_pet_waving_3,
};
static const uint16_t WAVING_DURATIONS[] = {140, 140, 140, 280};

static const lv_image_dsc_t *const FAILED_FRAMES[] = {
    &passport_pet_failed_0, &passport_pet_failed_1, &passport_pet_failed_2,
    &passport_pet_failed_3, &passport_pet_failed_4, &passport_pet_failed_5,
    &passport_pet_failed_6, &passport_pet_failed_7,
};
static const uint16_t FAILED_DURATIONS[] = {140, 140, 140, 140, 140, 140, 140, 240};

static const pet_animation_t IDLE = {
    IDLE_FRAMES, IDLE_DURATIONS, ARRAY_COUNT(IDLE_FRAMES), true,
};
static const pet_animation_t WAITING = {
    WAITING_FRAMES, WAITING_DURATIONS, ARRAY_COUNT(WAITING_FRAMES), true,
};
static const pet_animation_t RUNNING = {
    RUNNING_FRAMES, RUNNING_DURATIONS, ARRAY_COUNT(RUNNING_FRAMES), true,
};
static const pet_animation_t WAVING = {
    WAVING_FRAMES, WAVING_DURATIONS, ARRAY_COUNT(WAVING_FRAMES), false,
};
static const pet_animation_t FAILED = {
    FAILED_FRAMES, FAILED_DURATIONS, ARRAY_COUNT(FAILED_FRAMES), false,
};

static lv_obj_t *s_image;
static const pet_animation_t *s_animation = &WAITING;
static uint32_t s_started_at;
static uint8_t s_frame_index;

static const pet_animation_t *animation_for(passport_ui_state_t state)
{
    switch (state) {
    case PASSPORT_UI_SETUP: return &WAVING;
    case PASSPORT_UI_CONNECTING:
    case PASSPORT_UI_LISTENING:
    case PASSPORT_UI_BOOTING: return &WAITING;
    case PASSPORT_UI_THINKING: return &RUNNING;
    case PASSPORT_UI_ERROR: return &FAILED;
    case PASSPORT_UI_READY:
    case PASSPORT_UI_SPEAKING:
    default: return &IDLE;
    }
}

static uint8_t frame_at(const pet_animation_t *animation, uint32_t elapsed_ms)
{
    uint32_t total_ms = 0;
    for (uint8_t i = 0; i < animation->count; ++i) total_ms += animation->durations_ms[i];
    uint32_t position = animation->loop
                            ? elapsed_ms % total_ms
                            : (elapsed_ms < total_ms ? elapsed_ms : total_ms - 1);
    uint32_t boundary = 0;
    for (uint8_t i = 0; i < animation->count; ++i) {
        boundary += animation->durations_ms[i];
        if (position < boundary) return i;
    }
    return animation->count - 1;
}

static void animate(lv_timer_t *timer)
{
    (void)timer;
    if (!s_image || !s_animation) return;
    uint8_t next = frame_at(s_animation, lv_tick_elaps(s_started_at));
    if (next == s_frame_index) return;
    s_frame_index = next;
    lv_image_set_src(s_image, s_animation->frames[s_frame_index]);
}

void passport_pet_init(lv_obj_t *parent)
{
    s_image = lv_image_create(parent);
    lv_image_set_src(s_image, WAITING.frames[0]);
    lv_obj_align(s_image, LV_ALIGN_CENTER, 0, -24);
    s_started_at = lv_tick_get();
    s_frame_index = 0;
    lv_timer_create(animate, 30, NULL);
}

void passport_pet_set_state(passport_ui_state_t state)
{
    const pet_animation_t *next = animation_for(state);
    if (!s_image || next == s_animation) return;
    s_animation = next;
    s_started_at = lv_tick_get();
    s_frame_index = 0;
    lv_image_set_src(s_image, s_animation->frames[0]);
}
