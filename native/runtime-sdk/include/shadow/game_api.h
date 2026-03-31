#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint64_t EntityId;

typedef struct ShadowEngineCtx {
    void* editor_userdata;
    uint64_t frame_index;
} ShadowEngineCtx;

typedef struct ComponentMeta {
    const char* name;
    uint32_t size;
    uint32_t align;
} ComponentMeta;

void shadow_init(ShadowEngineCtx* ctx);
void shadow_update(float delta_time);
void shadow_shutdown(void);

uint32_t shadow_component_count(void);
ComponentMeta* shadow_component_meta(uint32_t index);
void* shadow_get_component(EntityId id, uint32_t component_type);
void shadow_set_component(EntityId id, uint32_t component_type, void* data);
void shadow_remove_component(EntityId id, uint32_t component_type);

EntityId shadow_create_entity(const char* name);
void shadow_destroy_entity(EntityId id);
void shadow_set_entity_name(EntityId id, const char* name);
void shadow_set_entity_scene_id(EntityId id, const char* scene_id);
EntityId shadow_find_entity_by_scene_id(const char* scene_id);
uint32_t shadow_entity_count(void);
EntityId* shadow_entity_list(void);

void shadow_load_scene(const char* path);
void shadow_save_scene(const char* path);

#ifdef __cplusplus
}
#endif
