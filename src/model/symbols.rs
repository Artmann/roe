use bitflags::bitflags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Class,
    Interface,
    Struct,
    Enum,
    Record,
    Delegate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberKind {
    Method,
    Constructor,
    StaticConstructor,
    Destructor,
    Property,
    Indexer,
    Field,
    EnumMember,
    Event,
    Operator,
    ConversionOperator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Type(TypeKind),
    Member(MemberKind),
    /// Synthetic per-file symbol that owns top-level statements, assembly
    /// attributes, and references from generated files.
    FileRoot,
}

impl SymbolKind {
    pub fn is_type(self) -> bool {
        matches!(self, SymbolKind::Type(_))
    }

    pub fn is_member(self) -> bool {
        matches!(self, SymbolKind::Member(_))
    }

    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Type(TypeKind::Class) => "class",
            SymbolKind::Type(TypeKind::Interface) => "interface",
            SymbolKind::Type(TypeKind::Struct) => "struct",
            SymbolKind::Type(TypeKind::Enum) => "enum",
            SymbolKind::Type(TypeKind::Record) => "record",
            SymbolKind::Type(TypeKind::Delegate) => "delegate",
            SymbolKind::Member(MemberKind::Method) => "method",
            SymbolKind::Member(MemberKind::Constructor) => "constructor",
            SymbolKind::Member(MemberKind::StaticConstructor) => "static constructor",
            SymbolKind::Member(MemberKind::Destructor) => "destructor",
            SymbolKind::Member(MemberKind::Property) => "property",
            SymbolKind::Member(MemberKind::Indexer) => "indexer",
            SymbolKind::Member(MemberKind::Field) => "field",
            SymbolKind::Member(MemberKind::EnumMember) => "enum member",
            SymbolKind::Member(MemberKind::Event) => "event",
            SymbolKind::Member(MemberKind::Operator) => "operator",
            SymbolKind::Member(MemberKind::ConversionOperator) => "conversion operator",
            SymbolKind::FileRoot => "file",
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifiers: u16 {
        const PUBLIC = 1;
        const INTERNAL = 1 << 1;
        const PROTECTED = 1 << 2;
        const PRIVATE = 1 << 3;
        const STATIC = 1 << 4;
        const PARTIAL = 1 << 5;
        const OVERRIDE = 1 << 6;
        const VIRTUAL = 1 << 7;
        const ABSTRACT = 1 << 8;
        const SEALED = 1 << 9;
        const READONLY = 1 << 10;
        const CONST = 1 << 11;
        const ASYNC = 1 << 12;
        const EXTERN = 1 << 13;
        const NEW = 1 << 14;
        const REQUIRED = 1 << 15;
    }
}

impl Modifiers {
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "public" => Some(Modifiers::PUBLIC),
            "internal" => Some(Modifiers::INTERNAL),
            "protected" => Some(Modifiers::PROTECTED),
            "private" => Some(Modifiers::PRIVATE),
            "static" => Some(Modifiers::STATIC),
            "partial" => Some(Modifiers::PARTIAL),
            "override" => Some(Modifiers::OVERRIDE),
            "virtual" => Some(Modifiers::VIRTUAL),
            "abstract" => Some(Modifiers::ABSTRACT),
            "sealed" => Some(Modifiers::SEALED),
            "readonly" => Some(Modifiers::READONLY),
            "const" => Some(Modifiers::CONST),
            "async" => Some(Modifiers::ASYNC),
            "extern" => Some(Modifiers::EXTERN),
            "new" => Some(Modifiers::NEW),
            "required" => Some(Modifiers::REQUIRED),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    Internal,
    Protected,
    ProtectedInternal,
    Private,
}

impl Visibility {
    /// C# defaults: top-level types are internal, members are private.
    pub fn from_modifiers(modifiers: Modifiers, is_top_level_type: bool) -> Self {
        let protected = modifiers.contains(Modifiers::PROTECTED);
        let internal = modifiers.contains(Modifiers::INTERNAL);
        if modifiers.contains(Modifiers::PUBLIC) {
            Visibility::Public
        } else if protected && internal {
            Visibility::ProtectedInternal
        } else if protected {
            Visibility::Protected
        } else if internal {
            Visibility::Internal
        } else if modifiers.contains(Modifiers::PRIVATE) {
            Visibility::Private
        } else if is_top_level_type {
            Visibility::Internal
        } else {
            Visibility::Private
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Internal => "internal",
            Visibility::Protected => "protected",
            Visibility::ProtectedInternal => "protected internal",
            Visibility::Private => "private",
        }
    }
}
