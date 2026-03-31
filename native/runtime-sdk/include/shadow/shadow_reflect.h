#pragma once
/**
 * ShadowEditor Component Reflection Macros
 *
 * These macros expand to NOTHING at compile time — the C++ compiler ignores them
 * completely. editor-header-tool (Rust) parses these annotations to generate
 * shadow_reflect.json, which the inspector reads to build its UI.
 *
 * Usage:
 *   #include "shadow/shadow_reflect.h"
 *
 *   SHADOW_COMPONENT()
 *   struct Health {
 *       SHADOW_PROPERTY(float, "display_name=Current HP, min=0, max=max")
 *       float current = 100.0f;
 *
 *       SHADOW_PROPERTY(float, "display_name=Max HP, min=1")
 *       float max = 100.0f;
 *
 *       SHADOW_PROPERTY(bool, "display_name=Regenerates")
 *       bool regen_enabled = false;
 *   };
 *
 * Supported SHADOW_PROPERTY metadata keys:
 *   display_name   Human-readable field label shown in inspector
 *   min            Numeric minimum (clamp in UI)
 *   max            Numeric maximum (clamp in UI)
 *   step           Drag increment for sliders
 *   group          Collapse group name
 *   tooltip        Hover tooltip text
 *   readonly       "true" → shown but not editable
 *   hidden         "true" → excluded from inspector entirely
 */

/* SHADOW_COMPONENT() — marks the following struct as an ECS component.
 * Place immediately before the struct keyword. */
#define SHADOW_COMPONENT()

/* SHADOW_PROPERTY(type, metadata) — marks the next field for inspector display.
 * Place immediately before the field declaration.
 * @param type     C++ type of the field (float, int, bool, vec3, …)
 * @param ...      Metadata string literal, comma-separated key=value pairs  */
#define SHADOW_PROPERTY(type, ...)
