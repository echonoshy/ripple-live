#include "passport_policy.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

static bool valid_hostname(const char *host, size_t length)
{
    if (length == 0 || length > 253 || host[0] == '.' || host[length - 1] == '.') return false;

    size_t label_length = 0;
    for (size_t i = 0; i < length; ++i) {
        unsigned char character = (unsigned char)host[i];
        if (character == '.') {
            if (label_length == 0 || host[i - 1] == '-') return false;
            label_length = 0;
            continue;
        }
        if (!isalnum(character) && character != '-') return false;
        if (label_length == 0 && character == '-') return false;
        if (++label_length > 63) return false;
    }
    return label_length > 0 && host[length - 1] != '-';
}

static bool valid_bracketed_ipv6(const char *host, size_t length)
{
    if (length < 4 || host[0] != '[' || host[length - 1] != ']') return false;
    bool has_colon = false;
    for (size_t i = 1; i + 1 < length; ++i) {
        unsigned char character = (unsigned char)host[i];
        if (character == ':') has_colon = true;
        if (!isxdigit(character) && character != ':' && character != '.') return false;
    }
    return has_colon;
}

bool passport_gateway_is_valid(const char *gateway)
{
    if (!gateway) return false;
    size_t length = strlen(gateway);
    if (length < 3 || length >= 96) return false;

    const char *separator = strrchr(gateway, ':');
    if (!separator || separator == gateway || separator[1] == '\0') return false;
    size_t host_length = (size_t)(separator - gateway);
    if (!valid_hostname(gateway, host_length) &&
        !valid_bracketed_ipv6(gateway, host_length)) {
        return false;
    }

    char *end = NULL;
    unsigned long port = strtoul(separator + 1, &end, 10);
    return end && *end == '\0' && port > 0 && port <= 65535;
}

bool passport_response_id_is_valid(const char *response_id)
{
    if (!response_id) return false;
    size_t length = strlen(response_id);
    if (length == 0 || length >= PASSPORT_RESPONSE_ID_SIZE) return false;
    for (size_t i = 0; i < length; ++i) {
        unsigned char character = (unsigned char)response_id[i];
        if (!isalnum(character) && character != '-' && character != '_') return false;
    }
    return true;
}

bool passport_response_id_matches(const char *active_id, const char *event_id)
{
    return passport_response_id_is_valid(active_id) &&
           passport_response_id_is_valid(event_id) && strcmp(active_id, event_id) == 0;
}

bool passport_output_chunk_is_valid(uint32_t sample_rate, size_t sample_count)
{
    return sample_rate == PASSPORT_OUTPUT_SAMPLE_RATE && sample_count > 0 &&
           sample_count <= PASSPORT_OUTPUT_MAX_SAMPLES;
}

uint32_t passport_audio_duration_ms(size_t sample_count, uint32_t sample_rate)
{
    if (sample_rate == 0) return 0;
    return (uint32_t)((sample_count * 1000U) / sample_rate);
}

bool passport_deadline_expired(int64_t now_us, int64_t deadline_us)
{
    return deadline_us > 0 && now_us >= deadline_us;
}
