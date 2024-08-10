use super::*;

pub fn load(
    mut commands: Commands,
    mut ctx: bevy_egui::EguiContexts,
    mut fps: ResMut<bevy_framepace::FramepaceSettings>,
) {
    let mut appdata = AppData {
        textures: true,
        rate: 60.0,
        ..default()
    };
    ctx.ctx_mut().memory_mut(|storage| {
        if let Some(config) = config()
            .map(|config| config.join("config.ron"))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|str| ron::from_str::<egui::util::IdTypeMap>(&str).ok())
        {
            storage.data = config
        }
        let data = &mut storage.data;
        fn retrieve<T: egui::util::id_type_map::SerializableAny>(
            val: &mut T,
            key: &str,
            data: &mut egui::util::IdTypeMap,
        ) {
            if let Some(inner) = data.get_persisted(egui::Id::new(key)) {
                *val = inner
            }
        }
        retrieve(&mut appdata.version, "VERSION", data);
        retrieve(&mut appdata.paks, "PAKS", data);
        retrieve(&mut appdata.aes, "AES", data);
        retrieve(&mut appdata.cache, "CACHE", data);
        retrieve(&mut appdata.textures, "TEXTURES", data);
        retrieve(&mut appdata.script, "SCRIPT", data);
        retrieve(&mut appdata.cap, "CAP", data);
        retrieve(&mut appdata.rate, "RATE", data);
    });
    fps.limiter = match appdata.cap {
        true => bevy_framepace::Limiter::from_framerate(appdata.rate),
        false => bevy_framepace::Limiter::Off,
    };
    commands.insert_resource(appdata);
}

pub fn write(mut ctx: bevy_egui::EguiContexts, appdata: Res<AppData>) {
    if !appdata.is_changed() {
        return;
    }
    use egui::Id;
    ctx.ctx_mut().memory_mut(|storage| {
        let storage = &mut storage.data;
        storage.insert_persisted(Id::new("VERSION"), appdata.version);
        storage.insert_persisted(Id::new("PAKS"), appdata.paks.clone());
        storage.insert_persisted(Id::new("AES"), appdata.aes.clone());
        storage.insert_persisted(Id::new("CACHE"), appdata.cache);
        storage.insert_persisted(Id::new("TEXTURES"), appdata.textures);
        storage.insert_persisted(Id::new("SCRIPT"), appdata.script.clone());
        storage.insert_persisted(Id::new("CAP"), appdata.cap);
        storage.insert_persisted(Id::new("RATE"), appdata.rate);
        if let Some(config) = config() {
            let _ = std::fs::create_dir_all(&config);
            if let Ok(data) = ron::to_string(&storage) {
                let _ = std::fs::write(config.join("config.ron"), data);
            }
        }
    })
}
