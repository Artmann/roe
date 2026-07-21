#[derive(Debug, PartialEq, Eq)]
pub struct SlnProject {
    pub name: String,
    /// Backslash-normalized path relative to the .sln directory.
    pub relative_path: String,
}

/// Parse the line-based .sln format. Only C# project entries are returned;
/// solution folders and other project types fall out naturally because their
/// path segment doesn't end in .csproj.
pub fn parse_sln(content: &str) -> Vec<SlnProject> {
    let mut projects = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("Project(") {
            continue;
        }
        // Project("{TYPE-GUID}") = "Name", "Rel\Path.csproj", "{PROJECT-GUID}"
        let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        if quoted.len() < 3 {
            continue;
        }
        let (name, path) = (quoted[1], quoted[2]);
        if !path.to_ascii_lowercase().ends_with(".csproj") {
            continue;
        }
        projects.push(SlnProject {
            name: name.to_string(),
            relative_path: path.replace('\\', "/"),
        });
    }

    projects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_entries_and_skips_solution_folders() {
        let sln = r#"
Microsoft Visual Studio Solution File, Format Version 12.00
# Visual Studio Version 17
Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "MyApp", "src\MyApp\MyApp.csproj", "{11111111-1111-1111-1111-111111111111}"
EndProject
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "Solution Items", "Solution Items", "{22222222-2222-2222-2222-222222222222}"
EndProject
Project("{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}") = "MyApp.Tests", "tests\MyApp.Tests\MyApp.Tests.csproj", "{33333333-3333-3333-3333-333333333333}"
EndProject
Project("{F184B08F-C81C-45F6-A57F-5ABD9991F28F}") = "VbThing", "vb\VbThing.vbproj", "{44444444-4444-4444-4444-444444444444}"
EndProject
Global
EndGlobal
"#;
        let projects = parse_sln(sln);
        assert_eq!(
            projects,
            vec![
                SlnProject {
                    name: "MyApp".to_string(),
                    relative_path: "src/MyApp/MyApp.csproj".to_string(),
                },
                SlnProject {
                    name: "MyApp.Tests".to_string(),
                    relative_path: "tests/MyApp.Tests/MyApp.Tests.csproj".to_string(),
                },
            ]
        );
    }

    #[test]
    fn tolerates_garbage_lines() {
        assert!(parse_sln("Project(\nProject(\"broken\nnot a project line\n").is_empty());
    }
}
