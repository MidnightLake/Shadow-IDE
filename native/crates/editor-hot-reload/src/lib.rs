use anyhow::{Context, Result, bail};
use libloading::{Library, Symbol};
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::slice;

pub type EntityId = u64;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ShadowEngineCtx {
    pub editor_userdata: *mut c_void,
    pub frame_index: u64,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ComponentMeta {
    pub name: *const c_char,
    pub size: u32,
    pub align: u32,
}

type ShadowInitFn = unsafe extern "C" fn(*mut ShadowEngineCtx);
type ShadowUpdateFn = unsafe extern "C" fn(f32);
type ShadowShutdownFn = unsafe extern "C" fn();
type ShadowLoadSceneFn = unsafe extern "C" fn(*const c_char);
type ShadowSaveSceneFn = unsafe extern "C" fn(*const c_char);
type ShadowComponentCountFn = unsafe extern "C" fn() -> u32;
type ShadowComponentMetaFn = unsafe extern "C" fn(u32) -> *mut ComponentMeta;
type ShadowGetComponentFn = unsafe extern "C" fn(EntityId, u32) -> *mut c_void;
type ShadowSetComponentFn = unsafe extern "C" fn(EntityId, u32, *mut c_void);
type ShadowRemoveComponentFn = unsafe extern "C" fn(EntityId, u32);
type ShadowCreateEntityFn = unsafe extern "C" fn(*const c_char) -> EntityId;
type ShadowDestroyEntityFn = unsafe extern "C" fn(EntityId);
type ShadowSetEntityNameFn = unsafe extern "C" fn(EntityId, *const c_char);
type ShadowSetEntitySceneIdFn = unsafe extern "C" fn(EntityId, *const c_char);
type ShadowFindEntityBySceneIdFn = unsafe extern "C" fn(*const c_char) -> EntityId;
type ShadowEntityCountFn = unsafe extern "C" fn() -> u32;
type ShadowEntityListFn = unsafe extern "C" fn() -> *mut EntityId;

#[derive(Debug, Clone)]
pub struct RuntimeComponentInfo {
    pub type_id: u32,
    pub name: String,
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPhase {
    WaitingForBuild,
    ReadyToLoad,
    Live,
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeApi {
    init: Option<ShadowInitFn>,
    update: Option<ShadowUpdateFn>,
    shutdown: Option<ShadowShutdownFn>,
    load_scene: Option<ShadowLoadSceneFn>,
    save_scene: Option<ShadowSaveSceneFn>,
    component_count: Option<ShadowComponentCountFn>,
    component_meta: Option<ShadowComponentMetaFn>,
    get_component: Option<ShadowGetComponentFn>,
    set_component: Option<ShadowSetComponentFn>,
    remove_component: Option<ShadowRemoveComponentFn>,
    create_entity: Option<ShadowCreateEntityFn>,
    destroy_entity: Option<ShadowDestroyEntityFn>,
    set_entity_name: Option<ShadowSetEntityNameFn>,
    set_entity_scene_id: Option<ShadowSetEntitySceneIdFn>,
    find_entity_by_scene_id: Option<ShadowFindEntityBySceneIdFn>,
    entity_count: Option<ShadowEntityCountFn>,
    entity_list: Option<ShadowEntityListFn>,
}

pub struct HotReloadHost {
    library_path: PathBuf,
    phase: HostPhase,
    detail: String,
    library: Option<Library>,
    api: RuntimeApi,
    ctx: ShadowEngineCtx,
    component_count: u32,
    entity_count: u32,
    component_defs: Vec<RuntimeComponentInfo>,
    entity_ids: Vec<EntityId>,
}

impl HotReloadHost {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let library_path = path.as_ref().to_path_buf();
        let (phase, detail) = if library_path.exists() {
            (
                HostPhase::ReadyToLoad,
                "Runtime artifact found and ready for the stable C ABI host.".to_string(),
            )
        } else {
            (
                HostPhase::WaitingForBuild,
                "Waiting for the first C++23 shared library build.".to_string(),
            )
        };

        Self {
            library_path,
            phase,
            detail,
            library: None,
            api: RuntimeApi::default(),
            ctx: ShadowEngineCtx::default(),
            component_count: 0,
            entity_count: 0,
            component_defs: Vec::new(),
            entity_ids: Vec::new(),
        }
    }

    pub fn library_path(&self) -> &Path {
        &self.library_path
    }

    pub fn status_line(&self) -> String {
        format!(
            "{:?} | {} | {} components | {} entities | frame {}",
            self.phase, self.detail, self.component_count, self.entity_count, self.ctx.frame_index
        )
    }

    pub fn is_live(&self) -> bool {
        matches!(self.phase, HostPhase::Live)
    }

    pub fn entity_count(&self) -> u32 {
        self.entity_count
    }

    pub fn component_count(&self) -> u32 {
        self.component_count
    }

    pub fn frame_index(&self) -> u64 {
        self.ctx.frame_index
    }

    pub fn component_types(&self) -> &[RuntimeComponentInfo] {
        &self.component_defs
    }

    pub fn component_type_by_name(&self, name: &str) -> Option<&RuntimeComponentInfo> {
        self.component_defs.iter().find(|component| component.name == name)
    }

    pub fn component_index_by_name(&self, name: &str) -> Option<u32> {
        self.component_type_by_name(name).map(|component| component.type_id)
    }

    pub fn entity_ids(&self) -> &[EntityId] {
        &self.entity_ids
    }

    pub fn load_if_present(&mut self) -> Result<()> {
        if !self.library_path.exists() {
            self.phase = HostPhase::WaitingForBuild;
            self.detail = format!("runtime library not found at {}", self.library_path.display());
            bail!("{}", self.detail);
        }

        self.shutdown_live_session();

        let library = unsafe { Library::new(&self.library_path) }.with_context(|| {
            format!(
                "failed to load runtime library {}",
                self.library_path.display()
            )
        })?;

        let api = unsafe { load_api(&library)? };
        self.library = Some(library);
        self.api = api;

        if let Some(init) = self.api.init {
            unsafe { init(&mut self.ctx) };
        }

        self.refresh_component_defs();
        self.refresh_counts();
        self.phase = HostPhase::Live;
        self.detail = "Runtime loaded and initialized across the stable C ABI boundary.".into();
        Ok(())
    }

    pub fn update(&mut self, delta_time: f32) -> Result<()> {
        let Some(update) = self.api.update else {
            bail!("runtime update function is not available");
        };
        self.ctx.frame_index = self.ctx.frame_index.saturating_add(1);
        unsafe { update(delta_time) };
        self.refresh_counts();
        self.detail = format!("Runtime updated via Play-in-Editor tick ({:.4} s).", delta_time);
        Ok(())
    }

    pub fn load_scene(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let Some(load_scene) = self.api.load_scene else {
            bail!("runtime scene load entry point is not available");
        };
        let path = CString::new(path.as_ref().to_string_lossy().to_string())
            .context("failed to build scene path CString")?;
        unsafe { load_scene(path.as_ptr()) };
        self.refresh_counts();
        self.detail = "Loaded .shadow scene into the live C++23 runtime.".into();
        Ok(())
    }

    pub fn save_scene(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let Some(save_scene) = self.api.save_scene else {
            bail!("runtime scene save entry point is not available");
        };
        let path = CString::new(path.as_ref().to_string_lossy().to_string())
            .context("failed to build scene path CString")?;
        unsafe { save_scene(path.as_ptr()) };
        self.detail = "Saved .shadow scene from the live C++23 runtime.".into();
        Ok(())
    }

    pub fn create_entity(&mut self, name: &str) -> Result<EntityId> {
        let Some(create_entity) = self.api.create_entity else {
            bail!("runtime create_entity entry point is not available");
        };
        let name = CString::new(name).context("failed to build entity name CString")?;
        let entity_id = unsafe { create_entity(name.as_ptr()) };
        self.refresh_counts();
        self.detail = format!("Created live runtime entity {}.", entity_id);
        Ok(entity_id)
    }

    pub fn destroy_entity(&mut self, entity_id: EntityId) -> Result<()> {
        let Some(destroy_entity) = self.api.destroy_entity else {
            bail!("runtime destroy_entity entry point is not available");
        };
        unsafe { destroy_entity(entity_id) };
        self.refresh_counts();
        self.detail = format!("Destroyed live runtime entity {}.", entity_id);
        Ok(())
    }

    pub fn remove_component(&mut self, entity_id: EntityId, component_type: u32) -> Result<bool> {
        let Some(remove_component) = self.api.remove_component else {
            return Ok(false);
        };
        unsafe { remove_component(entity_id, component_type) };
        if let Some(component) = self
            .component_defs
            .iter()
            .find(|component| component.type_id == component_type)
        {
            self.detail = format!(
                "Removed live runtime component {} from entity {}.",
                component.name, entity_id
            );
        }
        Ok(true)
    }

    pub fn set_entity_name(&mut self, entity_id: EntityId, name: &str) -> Result<bool> {
        let Some(set_entity_name) = self.api.set_entity_name else {
            return Ok(false);
        };
        let name = CString::new(name).context("failed to build entity name CString")?;
        unsafe { set_entity_name(entity_id, name.as_ptr()) };
        self.detail = format!("Renamed live runtime entity {}.", entity_id);
        Ok(true)
    }

    pub fn set_entity_scene_id(&mut self, entity_id: EntityId, scene_id: &str) -> Result<bool> {
        let Some(set_entity_scene_id) = self.api.set_entity_scene_id else {
            return Ok(false);
        };
        let scene_id = CString::new(scene_id).context("failed to build entity scene ID CString")?;
        unsafe { set_entity_scene_id(entity_id, scene_id.as_ptr()) };
        self.detail = format!("Bound live runtime entity {} to a scene ID.", entity_id);
        Ok(true)
    }

    pub fn find_entity_by_scene_id(&self, scene_id: &str) -> Result<Option<EntityId>> {
        let Some(find_entity_by_scene_id) = self.api.find_entity_by_scene_id else {
            return Ok(None);
        };
        let scene_id = CString::new(scene_id).context("failed to build entity scene ID CString")?;
        let entity_id = unsafe { find_entity_by_scene_id(scene_id.as_ptr()) };
        if entity_id == 0 {
            Ok(None)
        } else {
            Ok(Some(entity_id))
        }
    }

    pub fn get_component_bytes(&self, entity_id: EntityId, component_type: u32) -> Result<Option<Vec<u8>>> {
        let Some(get_component) = self.api.get_component else {
            bail!("runtime get_component entry point is not available");
        };
        let Some(component) = self
            .component_defs
            .iter()
            .find(|component| component.type_id == component_type)
        else {
            bail!("component type {} is not known to the host", component_type);
        };

        let ptr = unsafe { get_component(entity_id, component_type) };
        if ptr.is_null() {
            return Ok(None);
        }

        let bytes = unsafe { slice::from_raw_parts(ptr.cast::<u8>(), component.size as usize) };
        Ok(Some(bytes.to_vec()))
    }

    pub fn set_component_bytes(
        &mut self,
        entity_id: EntityId,
        component_type: u32,
        data: &[u8],
    ) -> Result<()> {
        let Some(set_component) = self.api.set_component else {
            bail!("runtime set_component entry point is not available");
        };
        let Some(component) = self
            .component_defs
            .iter()
            .find(|component| component.type_id == component_type)
        else {
            bail!("component type {} is not known to the host", component_type);
        };
        if data.len() != component.size as usize {
            bail!(
                "component blob size mismatch for {}: expected {}, got {}",
                component.name,
                component.size,
                data.len()
            );
        }

        let mut owned = data.to_vec();
        unsafe { set_component(entity_id, component_type, owned.as_mut_ptr().cast::<c_void>()) };
        self.detail = format!(
            "Updated live runtime component {} on entity {}.",
            component.name, entity_id
        );
        Ok(())
    }

    pub fn stop_session(&mut self) {
        self.shutdown_live_session();
        self.phase = if self.library_path.exists() {
            HostPhase::ReadyToLoad
        } else {
            HostPhase::WaitingForBuild
        };
        self.detail = "Runtime session stopped — editor back in authoring mode.".into();
    }

    fn refresh_counts(&mut self) {
        self.component_count = self
            .api
            .component_count
            .map(|count| unsafe { count() })
            .unwrap_or(0);
        self.entity_count = self
            .api
            .entity_count
            .map(|count| unsafe { count() })
            .unwrap_or(0);
        self.entity_ids = self
            .api
            .entity_list
            .and_then(|entity_list| {
                let ptr = unsafe { entity_list() };
                if ptr.is_null() || self.entity_count == 0 {
                    return Some(Vec::new());
                }
                let ids = unsafe { slice::from_raw_parts(ptr.cast::<EntityId>(), self.entity_count as usize) };
                Some(ids.to_vec())
            })
            .unwrap_or_default();
    }

    fn refresh_component_defs(&mut self) {
        let Some(component_meta) = self.api.component_meta else {
            self.component_defs.clear();
            return;
        };
        let count = self
            .api
            .component_count
            .map(|component_count| unsafe { component_count() })
            .unwrap_or(0);

        let mut component_defs = Vec::with_capacity(count as usize);
        for type_id in 0..count {
            let meta_ptr = unsafe { component_meta(type_id) };
            if meta_ptr.is_null() {
                continue;
            }
            let meta = unsafe { *meta_ptr };
            let name = if meta.name.is_null() {
                format!("Component{}", type_id)
            } else {
                unsafe { CStr::from_ptr(meta.name) }
                    .to_string_lossy()
                    .into_owned()
            };
            component_defs.push(RuntimeComponentInfo {
                type_id,
                name,
                size: meta.size,
                align: meta.align,
            });
        }
        self.component_defs = component_defs;
    }

    fn shutdown_live_session(&mut self) {
        if let Some(shutdown) = self.api.shutdown {
            unsafe { shutdown() };
        }
        self.api = RuntimeApi::default();
        self.library = None;
        self.component_count = 0;
        self.entity_count = 0;
        self.component_defs.clear();
        self.entity_ids.clear();
    }
}

impl Drop for HotReloadHost {
    fn drop(&mut self) {
        self.shutdown_live_session();
    }
}

unsafe fn load_api(library: &Library) -> Result<RuntimeApi> {
    Ok(RuntimeApi {
        init: Some(unsafe {
            copy_symbol(library.get::<ShadowInitFn>(b"shadow_init\0")?)
        }?),
        update: Some(unsafe {
            copy_symbol(library.get::<ShadowUpdateFn>(b"shadow_update\0")?)
        }?),
        shutdown: Some(unsafe {
            copy_symbol(library.get::<ShadowShutdownFn>(b"shadow_shutdown\0")?)
        }?),
        load_scene: unsafe { optional_symbol(library, b"shadow_load_scene\0") }?,
        save_scene: unsafe { optional_symbol(library, b"shadow_save_scene\0") }?,
        component_count: unsafe { optional_symbol(library, b"shadow_component_count\0") }?,
        component_meta: unsafe { optional_symbol(library, b"shadow_component_meta\0") }?,
        get_component: unsafe { optional_symbol(library, b"shadow_get_component\0") }?,
        set_component: unsafe { optional_symbol(library, b"shadow_set_component\0") }?,
        remove_component: unsafe { optional_symbol(library, b"shadow_remove_component\0") }?,
        create_entity: unsafe { optional_symbol(library, b"shadow_create_entity\0") }?,
        destroy_entity: unsafe { optional_symbol(library, b"shadow_destroy_entity\0") }?,
        set_entity_name: unsafe { optional_symbol(library, b"shadow_set_entity_name\0") }?,
        set_entity_scene_id: unsafe { optional_symbol(library, b"shadow_set_entity_scene_id\0") }?,
        find_entity_by_scene_id: unsafe { optional_symbol(library, b"shadow_find_entity_by_scene_id\0") }?,
        entity_count: unsafe { optional_symbol(library, b"shadow_entity_count\0") }?,
        entity_list: unsafe { optional_symbol(library, b"shadow_entity_list\0") }?,
    })
}

unsafe fn copy_symbol<T: Copy>(symbol: Symbol<'_, T>) -> Result<T> {
    Ok(*symbol)
}

unsafe fn optional_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<Option<T>> {
    match unsafe { library.get::<T>(name) } {
        Ok(symbol) => Ok(Some(*symbol)),
        Err(_) => Ok(None),
    }
}
