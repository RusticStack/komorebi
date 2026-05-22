#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryOwner {
    RustCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegedOperation {
    ServerAffectingMailAction,
    CredentialAccess,
    AccountMetadataMutation,
    AttachmentWrite,
    DestructiveDelete,
    Migration,
    SyncCacheMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationBoundary {
    pub operation: PrivilegedOperation,
    pub owner: BoundaryOwner,
    pub command_family: &'static str,
    pub rationale: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendSqlPolicy {
    Disabled,
    ReadOnly {
        allowed_tables: &'static [&'static str],
        justification: &'static str,
    },
}

pub const FRONTEND_SQL_POLICY: FrontendSqlPolicy = FrontendSqlPolicy::Disabled;

pub const PRIVILEGED_MUTATION_BOUNDARIES: &[MutationBoundary] = &[
    MutationBoundary {
        operation: PrivilegedOperation::ServerAffectingMailAction,
        owner: BoundaryOwner::RustCommand,
        command_family: "mail_action",
        rationale: "Remote flag, move, delete, archive, send, and draft operations need retry-aware server-state handling.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::CredentialAccess,
        owner: BoundaryOwner::RustCommand,
        command_family: "account_auth",
        rationale: "Credential Manager and OAuth tokens must never be exposed to React or direct SQL.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::AccountMetadataMutation,
        owner: BoundaryOwner::RustCommand,
        command_family: "account_metadata",
        rationale: "Account display metadata and signatures need ownership checks, sanitization, and typed persistence.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::AttachmentWrite,
        owner: BoundaryOwner::RustCommand,
        command_family: "attachment",
        rationale: "Attachment paths must be app-generated and validated before local writes or previews.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::DestructiveDelete,
        owner: BoundaryOwner::RustCommand,
        command_family: "delete",
        rationale: "Local and remote destructive actions need confirmation, ownership checks, and recovery semantics.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::Migration,
        owner: BoundaryOwner::RustCommand,
        command_family: "migration",
        rationale: "Schema changes must be transactional and versioned before any cache is opened.",
    },
    MutationBoundary {
        operation: PrivilegedOperation::SyncCacheMutation,
        owner: BoundaryOwner::RustCommand,
        command_family: "sync_cache",
        rationale: "IMAP UID, UIDVALIDITY, flags, expunges, and folder state must be mutated atomically by the sync engine.",
    },
];

pub const fn privileged_mutation_boundaries() -> &'static [MutationBoundary] {
    PRIVILEGED_MUTATION_BOUNDARIES
}

pub fn boundary_for(operation: PrivilegedOperation) -> &'static MutationBoundary {
    PRIVILEGED_MUTATION_BOUNDARIES
        .iter()
        .find(|boundary| boundary.operation == operation)
        .expect("privileged operation must have a boundary")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIRED_OPERATIONS: &[PrivilegedOperation] = &[
        PrivilegedOperation::ServerAffectingMailAction,
        PrivilegedOperation::CredentialAccess,
        PrivilegedOperation::AccountMetadataMutation,
        PrivilegedOperation::AttachmentWrite,
        PrivilegedOperation::DestructiveDelete,
        PrivilegedOperation::Migration,
        PrivilegedOperation::SyncCacheMutation,
    ];

    #[test]
    fn every_required_privileged_operation_has_a_boundary() {
        for operation in REQUIRED_OPERATIONS {
            let boundary = boundary_for(*operation);

            assert_eq!(boundary.operation, *operation);
            assert_eq!(boundary.owner, BoundaryOwner::RustCommand);
            assert!(!boundary.command_family.trim().is_empty());
            assert!(!boundary.rationale.trim().is_empty());
        }
    }

    #[test]
    fn no_privileged_operation_is_left_to_frontend_sql_or_plugin_access() {
        for boundary in privileged_mutation_boundaries() {
            assert_eq!(boundary.owner, BoundaryOwner::RustCommand);
        }
    }

    #[test]
    fn command_families_are_unique() {
        let boundaries = privileged_mutation_boundaries();

        for (index, boundary) in boundaries.iter().enumerate() {
            assert!(
                boundaries[index + 1..]
                    .iter()
                    .all(|other| other.command_family != boundary.command_family),
                "duplicate command family: {}",
                boundary.command_family
            );
        }
    }

    #[test]
    fn frontend_sql_is_disabled_until_read_only_scope_is_documented() {
        assert_eq!(FRONTEND_SQL_POLICY, FrontendSqlPolicy::Disabled);
    }
}
