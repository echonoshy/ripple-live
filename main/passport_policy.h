#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define PASSPORT_INPUT_SAMPLE_RATE 16000U
#define PASSPORT_INPUT_SAMPLES 640U
#define PASSPORT_OUTPUT_SAMPLE_RATE 24000U
#define PASSPORT_OUTPUT_MAX_SAMPLES 2400U
#define PASSPORT_PLAYBACK_PREBUFFER_MS 400U
#define PASSPORT_RESPONSE_ID_SIZE 64U

bool passport_gateway_is_valid(const char *gateway);
bool passport_response_id_is_valid(const char *response_id);
bool passport_response_id_matches(const char *active_id, const char *event_id);
bool passport_output_chunk_is_valid(uint32_t sample_rate, size_t sample_count);
uint32_t passport_audio_duration_ms(size_t sample_count, uint32_t sample_rate);
bool passport_deadline_expired(int64_t now_us, int64_t deadline_us);
