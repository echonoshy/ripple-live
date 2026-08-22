file(GLOB PET_ASSET_SRCS "pet_assets/*.c")

idf_component_register(
    SRCS "main.c"
         "passport_ui.c"
         "passport_pet.c"
         "passport_wifi.c"
         "passport_realtime.c"
         ${PET_ASSET_SRCS}
    INCLUDE_DIRS "."
    REQUIRES bsp esp_wifi esp_event esp_netif nvs_flash esp_http_server json mbedtls
)

# LVGL's image converter uses this macro to select the component's simple
# include layout. ESP-IDF already exposes lvgl.h directly to this target.
target_compile_definitions(${COMPONENT_LIB} PRIVATE LV_LVGL_H_INCLUDE_SIMPLE)
