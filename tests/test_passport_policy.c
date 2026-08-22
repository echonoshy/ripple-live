#include <assert.h>
#include <stdint.h>

#include "passport_policy.h"

int main(void)
{
    assert(passport_gateway_is_valid("140.143.229.103:8700"));
    assert(passport_gateway_is_valid("gateway.local:443"));
    assert(passport_gateway_is_valid("[::1]:8700"));
    assert(!passport_gateway_is_valid("ws://gateway.local:8700"));
    assert(!passport_gateway_is_valid("gateway.local"));
    assert(!passport_gateway_is_valid("gateway.local:0"));
    assert(!passport_gateway_is_valid("gateway.local:65536"));
    assert(!passport_gateway_is_valid("bad host:8700"));

    assert(passport_response_id_is_valid("8a5e2c5b-1"));
    assert(!passport_response_id_is_valid(""));
    assert(!passport_response_id_is_valid("response/id"));
    assert(passport_response_id_matches("response-1", "response-1"));
    assert(!passport_response_id_matches("response-1", "response-2"));

    assert(passport_output_chunk_is_valid(24000, 2400));
    assert(passport_output_chunk_is_valid(24000, 1));
    assert(!passport_output_chunk_is_valid(16000, 2400));
    assert(!passport_output_chunk_is_valid(24000, 0));
    assert(!passport_output_chunk_is_valid(24000, 2401));
    assert(passport_audio_duration_ms(9600, 24000) == 400);
    assert(passport_audio_duration_ms(2400, 24000) == 100);
    assert(passport_audio_duration_ms(100, 0) == 0);

    assert(!passport_deadline_expired(100, 0));
    assert(!passport_deadline_expired(99, 100));
    assert(passport_deadline_expired(100, 100));
    assert(passport_deadline_expired(INT64_MAX, 100));
    return 0;
}
