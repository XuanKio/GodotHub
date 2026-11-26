//! Contents of new projects: the project file plus an optional starter scene.
use serde::Deserialize;

pub const MAIN_SCENE: &str = "main.tscn";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Template {
    #[default]
    Empty,
    TwoD,
    ThreeD,
    Ui,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Renderer {
    ForwardPlus,
    Mobile,
    Compatibility,
    Gles3,
    Gles2,
}

pub struct Contents {
    pub file_name: &'static str,
    pub project: String,
    pub scene: Option<String>,
}

/// `minor` is the engine's minor version; `dotnet` adds the C# feature.
pub fn contents(
    name: &str,
    major: u8,
    minor: u32,
    dotnet: bool,
    template: Template,
    renderer: Option<Renderer>,
) -> Result<Contents, String> {
    let title = serde_json::to_string(name).expect("a project name can always be encoded as text");
    match major {
        2 => {
            if template != Template::Empty || renderer.is_some() {
                return Err("Godot 2 projects can only be created empty.".into());
            }
            Ok(Contents {
                file_name: "engine.cfg",
                project: format!("[application]\nname={title}\n"),
                scene: None,
            })
        }
        3 => godot_3(&title, minor, dotnet, template, renderer),
        4 => godot_4(&title, minor, dotnet, template, renderer),
        _ => Err("Choose a Godot 2, 3 or 4 engine to create a project.".into()),
    }
}

fn godot_3(
    title: &str,
    minor: u32,
    dotnet: bool,
    template: Template,
    renderer: Option<Renderer>,
) -> Result<Contents, String> {
    let original = minor == 0;
    let gles2 = match (renderer, original) {
        (None, original) => !original,
        (Some(Renderer::Gles3), _) => false,
        (Some(Renderer::Gles2), false) => true,
        (Some(Renderer::Gles2), true) => {
            return Err("Godot 3.0 supports only the GLES3 renderer.".into())
        }
        _ => return Err("Godot 3 projects use the GLES3 or GLES2 renderer.".into()),
    };
    let version = if original { 3 } else { 4 };
    let features = if dotnet { "custom_features=\"C#\"\n" } else { "" };
    let mut body = format!("config_version={version}\n{features}\n[application]\nconfig/name={title}\n");
    if template != Template::Empty {
        body.push_str(&format!("run/main_scene=\"res://{MAIN_SCENE}\"\n"));
    }
    if matches!(template, Template::TwoD | Template::Ui) {
        body.push_str("\n[display]\nwindow/stretch/mode=\"2d\"\nwindow/stretch/aspect=\"keep\"\n");
    }
    if gles2 {
        body.push_str("\n[rendering]\nquality/driver/driver_name=\"GLES2\"\n");
    }
    Ok(Contents {
        file_name: "project.godot",
        project: body,
        scene: scene_3(template, title),
    })
}

fn godot_4(
    title: &str,
    minor: u32,
    dotnet: bool,
    template: Template,
    renderer: Option<Renderer>,
) -> Result<Contents, String> {
    let (feature, method) = match renderer.unwrap_or(Renderer::Compatibility) {
        Renderer::ForwardPlus => ("Forward Plus", None),
        Renderer::Mobile => ("Mobile", Some("mobile")),
        Renderer::Compatibility => ("GL Compatibility", Some("gl_compatibility")),
        _ => return Err("Godot 4 projects use Forward+, Mobile or Compatibility.".into()),
    };
    let dotnet = if dotnet { ", \"C#\"" } else { "" };
    let mut body = format!("config_version=5\n\n[application]\nconfig/name={title}\n");
    if template != Template::Empty {
        body.push_str(&format!("run/main_scene=\"res://{MAIN_SCENE}\"\n"));
    }
    body.push_str(&format!(
        "config/features=PackedStringArray(\"4.{minor}\"{dotnet}, \"{feature}\")\n"
    ));
    if matches!(template, Template::TwoD | Template::Ui) {
        body.push_str("\n[display]\nwindow/stretch/mode=\"canvas_items\"\n");
    }
    if let Some(method) = method {
        body.push_str(&format!(
            "\n[rendering]\nrenderer/rendering_method=\"{method}\"\n"
        ));
        if method == "gl_compatibility" {
            body.push_str("renderer/rendering_method.mobile=\"gl_compatibility\"\n");
        }
    }
    Ok(Contents {
        file_name: "project.godot",
        project: body,
        scene: scene_4(template, title),
    })
}

fn scene_4(template: Template, title: &str) -> Option<String> {
    const FULL_RECT: &str = "anchors_preset = 15\nanchor_right = 1.0\nanchor_bottom = 1.0\ngrow_horizontal = 2\ngrow_vertical = 2\n";
    Some(match template {
        Template::Empty => return None,
        Template::TwoD => "[gd_scene format=3]\n\n[node name=\"Main\" type=\"Node2D\"]\n\n[node name=\"Camera2D\" type=\"Camera2D\" parent=\".\"]\n".into(),
        Template::ThreeD => "[gd_scene format=3]\n\n\
[sub_resource type=\"ProceduralSkyMaterial\" id=\"SkyMaterial\"]\n\n\
[sub_resource type=\"Sky\" id=\"Sky\"]\nsky_material = SubResource(\"SkyMaterial\")\n\n\
[sub_resource type=\"Environment\" id=\"Environment\"]\nbackground_mode = 2\nsky = SubResource(\"Sky\")\n\n\
[node name=\"Main\" type=\"Node3D\"]\n\n\
[node name=\"WorldEnvironment\" type=\"WorldEnvironment\" parent=\".\"]\nenvironment = SubResource(\"Environment\")\n\n\
[node name=\"DirectionalLight3D\" type=\"DirectionalLight3D\" parent=\".\"]\nrotation = Vector3(-0.785398, 0.523599, 0)\nshadow_enabled = true\n\n\
[node name=\"Camera3D\" type=\"Camera3D\" parent=\".\"]\nposition = Vector3(0, 1.5, 4)\n".into(),
        Template::Ui => format!("[gd_scene format=3]\n\n\
[node name=\"Main\" type=\"Control\"]\nlayout_mode = 3\n{FULL_RECT}\n\
[node name=\"Margin\" type=\"MarginContainer\" parent=\".\"]\nlayout_mode = 1\n{FULL_RECT}\
theme_override_constants/margin_left = 32\ntheme_override_constants/margin_top = 32\ntheme_override_constants/margin_right = 32\ntheme_override_constants/margin_bottom = 32\n\n\
[node name=\"Content\" type=\"VBoxContainer\" parent=\"Margin\"]\nlayout_mode = 2\n\n\
[node name=\"Title\" type=\"Label\" parent=\"Margin/Content\"]\nlayout_mode = 2\ntext = {title}\n"),
    })
}

fn scene_3(template: Template, title: &str) -> Option<String> {
    const FULL_RECT: &str = "anchor_right = 1.0\nanchor_bottom = 1.0\n";
    Some(match template {
        Template::Empty => return None,
        Template::TwoD => "[gd_scene format=2]\n\n[node name=\"Main\" type=\"Node2D\"]\n\n[node name=\"Camera2D\" type=\"Camera2D\" parent=\".\"]\ncurrent = true\n".into(),
        Template::ThreeD => "[gd_scene load_steps=3 format=2]\n\n\
[sub_resource type=\"ProceduralSky\" id=1]\n\n\
[sub_resource type=\"Environment\" id=2]\nbackground_mode = 2\nbackground_sky = SubResource( 1 )\n\n\
[node name=\"Main\" type=\"Spatial\"]\n\n\
[node name=\"WorldEnvironment\" type=\"WorldEnvironment\" parent=\".\"]\nenvironment = SubResource( 2 )\n\n\
[node name=\"DirectionalLight\" type=\"DirectionalLight\" parent=\".\"]\nrotation_degrees = Vector3( -45, 30, 0 )\nshadow_enabled = true\n\n\
[node name=\"Camera\" type=\"Camera\" parent=\".\"]\ntranslation = Vector3( 0, 1.5, 4 )\n".into(),
        Template::Ui => format!("[gd_scene format=2]\n\n\
[node name=\"Main\" type=\"Control\"]\n{FULL_RECT}\n\
[node name=\"Margin\" type=\"MarginContainer\" parent=\".\"]\n{FULL_RECT}\
custom_constants/margin_right = 32\ncustom_constants/margin_top = 32\ncustom_constants/margin_left = 32\ncustom_constants/margin_bottom = 32\n\n\
[node name=\"Content\" type=\"VBoxContainer\" parent=\"Margin\"]\n\n\
[node name=\"Title\" type=\"Label\" parent=\"Margin/Content\"]\ntext = {title}\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_template_scenes_and_renderers_per_major() {
        let four = contents("Game", 4, 3, false, Template::ThreeD, Some(Renderer::ForwardPlus)).unwrap();
        assert!(four.project.contains("run/main_scene=\"res://main.tscn\""));
        assert!(four.project.contains("\"4.3\", \"Forward Plus\""));
        assert!(!four.project.contains("rendering_method"));
        assert!(four.scene.unwrap().contains("type=\"Camera3D\""));

        let mobile = contents("UI", 4, 4, true, Template::Ui, Some(Renderer::Mobile)).unwrap();
        assert!(mobile.project.contains("\"4.4\", \"C#\", \"Mobile\""));
        assert!(mobile.project.contains("rendering_method=\"mobile\""));
        assert!(mobile.project.contains("canvas_items"));
        assert!(mobile.scene.unwrap().contains("text = \"UI\""));

        let empty = contents("E", 4, 2, false, Template::Empty, None).unwrap();
        assert!(empty.scene.is_none());
        assert!(!empty.project.contains("main_scene"));
        assert!(empty.project.contains("gl_compatibility"));

        let three = contents("Old", 3, 6, false, Template::TwoD, Some(Renderer::Gles3)).unwrap();
        assert!(!three.project.contains("GLES2"));
        assert!(three.scene.unwrap().contains("format=2"));
        assert!(contents("Old", 3, 0, false, Template::Empty, Some(Renderer::Gles2)).is_err());
        assert!(contents("Bad", 4, 3, false, Template::Empty, Some(Renderer::Gles2)).is_err());
        assert!(contents("Bad", 3, 6, false, Template::Empty, Some(Renderer::Mobile)).is_err());
        assert!(contents("Two", 2, 1, false, Template::TwoD, None).is_err());
    }
}
