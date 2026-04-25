use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderProfileId {
    #[default]
    Diffuse,
    ArchitecturalV1,
    ArchitecturalV2,
    ArchitecturalV3,
    ArchitecturalV4,
}

impl RenderProfileId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Diffuse => "diffuse",
            Self::ArchitecturalV1 => "architectural-v1",
            Self::ArchitecturalV2 => "architectural-v2",
            Self::ArchitecturalV3 => "architectural-v3",
            Self::ArchitecturalV4 => "architectural-v4",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Diffuse => "Diffuse",
            Self::ArchitecturalV1 => "Architectural v1",
            Self::ArchitecturalV2 => "Architectural v2",
            Self::ArchitecturalV3 => "Architectural v3",
            Self::ArchitecturalV4 => "Architectural v4 (AO experiment)",
        }
    }

    pub const fn descriptor(self) -> RenderProfileDescriptor {
        RenderProfileDescriptor {
            id: self,
            name: self.as_str(),
            label: self.label(),
        }
    }
}

impl FromStr for RenderProfileId {
    type Err = UnknownRenderProfile;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "diffuse" => Ok(Self::Diffuse),
            "architectural-v1" => Ok(Self::ArchitecturalV1),
            "architectural-v2" => Ok(Self::ArchitecturalV2),
            "architectural-v3" => Ok(Self::ArchitecturalV3),
            "architectural-v4" => Ok(Self::ArchitecturalV4),
            _ => Err(UnknownRenderProfile {
                requested: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderProfileDescriptor {
    pub id: RenderProfileId,
    pub name: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown render profile `{requested}`")]
pub struct UnknownRenderProfile {
    requested: String,
}

const AVAILABLE_RENDER_PROFILES: [RenderProfileDescriptor; 5] = [
    RenderProfileId::Diffuse.descriptor(),
    RenderProfileId::ArchitecturalV1.descriptor(),
    RenderProfileId::ArchitecturalV2.descriptor(),
    RenderProfileId::ArchitecturalV3.descriptor(),
    RenderProfileId::ArchitecturalV4.descriptor(),
];

pub const fn available_render_profiles() -> &'static [RenderProfileDescriptor] {
    &AVAILABLE_RENDER_PROFILES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_round_trip() {
        for descriptor in available_render_profiles() {
            assert_eq!(
                descriptor.name.parse::<RenderProfileId>(),
                Ok(descriptor.id)
            );
            assert_eq!(descriptor.id.as_str(), descriptor.name);
            assert_eq!(descriptor.id.label(), descriptor.label);
        }
    }

    #[test]
    fn unknown_profile_is_rejected() {
        assert!("architectural".parse::<RenderProfileId>().is_err());
    }
}
