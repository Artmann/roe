use anyhow::Context;

use crate::model::{ProjectKind, TestFramework};

/// Raw data pulled from a .csproj. MSBuild `Condition` attributes are
/// deliberately ignored — we take all branches, because over-including
/// sources and references only ever marks more code as used (the safe
/// direction).
#[derive(Debug, Default)]
pub struct CsprojData {
    pub sdk: Option<String>,
    pub output_type: Option<String>,
    pub is_sdk_style: bool,
    pub enable_default_compile_items: bool,
    pub implicit_usings: bool,
    /// Raw ProjectReference Include values, backslash-normalized.
    pub project_refs: Vec<String>,
    pub package_refs: Vec<String>,
    /// Compile Include / Remove globs, backslash-normalized.
    pub compile_includes: Vec<String>,
    pub compile_removes: Vec<String>,
    /// <Using Include="..."/> global usings.
    pub usings: Vec<String>,
    /// Explicit <IsPackable> value, if present.
    pub is_packable: Option<bool>,
    /// Any NuGet packaging property (PackageId, PackageReleaseNotes, ...)
    /// was found — this project ships as a package.
    pub has_package_metadata: bool,
}

pub fn parse_csproj(content: &str) -> anyhow::Result<CsprojData> {
    let doc = roxmltree::Document::parse(content).context("invalid XML")?;
    let root = doc.root_element();

    let mut data = CsprojData {
        sdk: root.attribute("Sdk").map(str::to_string),
        enable_default_compile_items: true,
        ..CsprojData::default()
    };

    for node in root.descendants().filter(roxmltree::Node::is_element) {
        let tag = node.tag_name().name();
        match tag {
            // <Sdk Name="..."/> element form and <Import Sdk="..."/> both mark
            // SDK-style projects when the root attribute is absent.
            "Sdk" => {
                if data.sdk.is_none() {
                    data.sdk = node.attribute("Name").map(str::to_string);
                }
            }
            "Import" => {
                if data.sdk.is_none() {
                    data.sdk = node.attribute("Sdk").map(str::to_string);
                }
            }
            "OutputType" => data.output_type = node.text().map(|t| t.trim().to_string()),
            "EnableDefaultCompileItems" => {
                data.enable_default_compile_items = !node
                    .text()
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("false"));
            }
            "ImplicitUsings" => {
                data.implicit_usings = node.text().is_some_and(|t| {
                    let t = t.trim();
                    t.eq_ignore_ascii_case("enable") || t.eq_ignore_ascii_case("true")
                });
            }
            "ProjectReference" => {
                if let Some(include) = node.attribute("Include") {
                    data.project_refs.push(include.replace('\\', "/"));
                }
            }
            "PackageReference" => {
                if let Some(include) = node.attribute("Include") {
                    data.package_refs.push(include.to_string());
                }
            }
            "Compile" => {
                if let Some(include) = node.attribute("Include") {
                    data.compile_includes.push(include.replace('\\', "/"));
                }
                if let Some(remove) = node.attribute("Remove") {
                    data.compile_removes.push(remove.replace('\\', "/"));
                }
            }
            "Using" => {
                if let Some(include) = node.attribute("Include") {
                    data.usings.push(include.to_string());
                }
            }
            "IsPackable" => {
                data.is_packable = node.text().map(|t| !t.trim().eq_ignore_ascii_case("false"));
            }
            // PackageId/PackageVersion/PackageReleaseNotes/... are packaging
            // properties; PackageReference (and central-version PackageVersion
            // items) carry an Include attribute and are not.
            tag if tag.starts_with("Package") && node.attribute("Include").is_none() => {
                data.has_package_metadata = true;
            }
            _ => {}
        }
    }

    data.is_sdk_style = data.sdk.is_some();
    // Old-style projects enumerate every source explicitly.
    if !data.is_sdk_style {
        data.enable_default_compile_items = false;
    }

    Ok(data)
}

impl CsprojData {
    pub fn kind(&self) -> ProjectKind {
        if self
            .sdk
            .as_deref()
            .is_some_and(|sdk| sdk.contains("Microsoft.NET.Sdk.Web"))
        {
            return ProjectKind::Web;
        }
        match self.output_type.as_deref() {
            Some(t) if t.eq_ignore_ascii_case("Exe") || t.eq_ignore_ascii_case("WinExe") => {
                ProjectKind::Exe
            }
            _ => ProjectKind::Library,
        }
    }

    /// Ships as a NuGet package → its public API is an external contract.
    pub fn is_packable(&self) -> bool {
        self.is_packable.unwrap_or(self.has_package_metadata)
    }

    pub fn test_framework(&self) -> Option<TestFramework> {
        for package in &self.package_refs {
            let package = package.to_ascii_lowercase();
            if package.starts_with("xunit") {
                return Some(TestFramework::Xunit);
            }
            if package.starts_with("nunit") {
                return Some(TestFramework::Nunit);
            }
            if package.starts_with("mstest") {
                return Some(TestFramework::Mstest);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sdk_style_exe() {
        let data = parse_csproj(
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
  </PropertyGroup>
  <ItemGroup>
    <ProjectReference Include="..\Lib\Lib.csproj" />
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();

        assert_eq!(data.kind(), ProjectKind::Exe);
        assert!(data.is_sdk_style);
        assert!(data.implicit_usings);
        assert!(data.enable_default_compile_items);
        assert_eq!(data.project_refs, vec!["../Lib/Lib.csproj"]);
        assert_eq!(data.package_refs, vec!["Newtonsoft.Json"]);
        assert_eq!(data.test_framework(), None);
    }

    #[test]
    fn parses_web_sdk_and_test_frameworks() {
        let web = parse_csproj(r#"<Project Sdk="Microsoft.NET.Sdk.Web"></Project>"#).unwrap();
        assert_eq!(web.kind(), ProjectKind::Web);

        let tests = parse_csproj(
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="xunit.v3" Version="1.0.0" />
    <PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.0.0" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();
        assert_eq!(tests.test_framework(), Some(TestFramework::Xunit));
        assert_eq!(tests.kind(), ProjectKind::Library);
    }

    #[test]
    fn parses_compile_items_and_usings() {
        let data = parse_csproj(
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <Compile Include="..\Shared\Version.cs" />
    <Compile Remove="Legacy\**\*.cs" />
    <Using Include="MyApp.Common" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();

        assert_eq!(data.compile_includes, vec!["../Shared/Version.cs"]);
        assert_eq!(data.compile_removes, vec!["Legacy/**/*.cs"]);
        assert_eq!(data.usings, vec!["MyApp.Common"]);
    }

    #[test]
    fn old_style_project_disables_default_items() {
        let data = parse_csproj(
            r#"<Project ToolsVersion="15.0" xmlns="http://schemas.microsoft.com/developer/msbuild/2003">
  <PropertyGroup>
    <OutputType>Library</OutputType>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="Class1.cs" />
  </ItemGroup>
</Project>"#,
        )
        .unwrap();

        assert!(!data.is_sdk_style);
        assert!(!data.enable_default_compile_items);
        assert_eq!(data.compile_includes, vec!["Class1.cs"]);
    }

    #[test]
    fn rejects_invalid_xml() {
        assert!(parse_csproj("<Project><Unclosed></Project>").is_err());
    }
}
