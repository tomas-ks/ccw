use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderProfileId {
    Diffuse,
    #[default]
    Bim,
    Architectural,
    ArchitecturalV1,
    ArchitecturalV3,
    ArchitecturalV4,
}

impl RenderProfileId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Diffuse => "diffuse",
            Self::Bim => "bim",
            Self::Architectural => "architectural",
            Self::ArchitecturalV1 => "architectural-v1",
            Self::ArchitecturalV3 => "architectural-v3",
            Self::ArchitecturalV4 => "architectural-v4",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Diffuse => "Diffuse",
            Self::Bim => "BIM",
            Self::Architectural => "Architectural",
            Self::ArchitecturalV1 => "Architectural v1 (experimental)",
            Self::ArchitecturalV3 => "Architectural v3 (experimental)",
            Self::ArchitecturalV4 => "Architectural v4 (AO experiment)",
        }
    }

    pub const fn is_experimental(self) -> bool {
        match self {
            Self::Diffuse | Self::Bim | Self::Architectural => false,
            Self::ArchitecturalV1 | Self::ArchitecturalV3 | Self::ArchitecturalV4 => true,
        }
    }

    pub const fn descriptor(self) -> RenderProfileDescriptor {
        RenderProfileDescriptor {
            id: self,
            name: self.as_str(),
            label: self.label(),
            experimental: self.is_experimental(),
        }
    }
}

impl FromStr for RenderProfileId {
    type Err = UnknownRenderProfile;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "diffuse" => Ok(Self::Diffuse),
            "bim" | "architectural-v2" => Ok(Self::Bim),
            "architectural" | "architectural-v3-inspection" | "architectural-v3-inspect" => {
                Ok(Self::Architectural)
            }
            "architectural-v1" => Ok(Self::ArchitecturalV1),
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
    pub experimental: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown render profile `{requested}`")]
pub struct UnknownRenderProfile {
    requested: String,
}

const AVAILABLE_RENDER_PROFILES: [RenderProfileDescriptor; 6] = [
    RenderProfileId::Diffuse.descriptor(),
    RenderProfileId::Bim.descriptor(),
    RenderProfileId::Architectural.descriptor(),
    RenderProfileId::ArchitecturalV1.descriptor(),
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
        assert!(
            "architectural-v2-inspection"
                .parse::<RenderProfileId>()
                .is_err()
        );
    }

    #[test]
    fn legacy_profile_names_alias_to_new_stable_profiles() {
        assert_eq!("architectural-v2".parse(), Ok(RenderProfileId::Bim));
        assert_eq!(
            "architectural-v3-inspection".parse(),
            Ok(RenderProfileId::Architectural)
        );
        assert_eq!(
            "architectural-v3-inspect".parse(),
            Ok(RenderProfileId::Architectural)
        );
    }
}
