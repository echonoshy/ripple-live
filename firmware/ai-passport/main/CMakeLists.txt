idf_component_register(
    SRCS "main.c"
         "passport_ui.c"
         "passport_wifi.c"
         "passport_realtime.c"
    INCLUDE_DIRS "."
    REQUIRES bsp esp_wifi esp_event esp_netif nvs_flash esp_http_server json mbedtls
)
