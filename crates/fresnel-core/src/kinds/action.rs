//! The actions on an object (plan section 10). In M2, an action is a name: the command
//! table gives it a key, and the operations of later milestones give it a request.

/// One action on the objects of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Shows the manifest of the object, read-only (plan section 12.9).
    ViewManifest,
    /// Opens the manifest in the editor (M11).
    EditManifest,
    /// Deletes the object, with a propagation policy.
    Delete,
    /// Deletes the object with no grace period.
    ForceDelete,
    /// Saves the manifest of the object as a file.
    Download,
    /// Copies the name of the object.
    CopyName,
    /// Copies the `kubectl` command that reads the object.
    CopyCommand,
    /// Sets the count of pods.
    Scale,
    /// Starts each pod again, one after the other.
    Restart,
    /// Shows the progress of a rollout.
    RolloutStatus,
    /// Shows the earlier revisions of the pod template.
    RevisionHistory,
    /// Goes back to an earlier revision.
    Rollback,
    /// Evicts a pod through the eviction API, which obeys each PodDisruptionBudget.
    Evict,
    /// Shows the logs (M9).
    Logs,
    /// Opens a shell in a container (M10).
    Terminal,
    /// Attaches to the main process of a container (M10).
    Attach,
    /// Adds a debug container to a pod (M10).
    DebugContainer,
    /// Forwards a local port to a port of the object (plan section 12.3).
    Forward,
    /// Makes a new Job from a Job that ended.
    RunAgain,
    /// Stops the start of new pods or of new Jobs.
    Suspend,
    /// Ends a suspend.
    Resume,
    /// Makes a Job from a CronJob now.
    Trigger,
    /// Closes a node for new pods.
    Cordon,
    /// Opens a node for new pods again.
    Uncordon,
    /// Evicts each pod of a node, as a visible task.
    Drain,
    /// Opens a shell on a node (M10).
    NodeShell,
    /// Changes the value of a key of a Secret or of a ConfigMap (M4).
    EditKeys,
    /// Adds a key (M4).
    AddKey,
    /// Removes a key (M4).
    RemoveKey,
}

impl Action {
    /// The actions on an object of each kind. A kind adds its own ones.
    pub const EACH_KIND: [Self; 7] = [
        Self::ViewManifest,
        Self::EditManifest,
        Self::Delete,
        Self::ForceDelete,
        Self::Download,
        Self::CopyName,
        Self::CopyCommand,
    ];

    /// The words of a menu entry.
    pub fn label(self) -> &'static str {
        match self {
            Self::ViewManifest => "View manifest",
            Self::EditManifest => "Edit manifest",
            Self::Delete => "Delete",
            Self::ForceDelete => "Force delete",
            Self::Download => "Download",
            Self::CopyName => "Copy name",
            Self::CopyCommand => "Copy kubectl command",
            Self::Scale => "Scale",
            Self::Restart => "Restart",
            Self::RolloutStatus => "Rollout status",
            Self::RevisionHistory => "Revision history",
            Self::Rollback => "Roll back",
            Self::Evict => "Evict",
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Attach => "Attach",
            Self::DebugContainer => "Debug container",
            Self::Forward => "Forward a port",
            Self::RunAgain => "Run again",
            Self::Suspend => "Suspend",
            Self::Resume => "Resume",
            Self::Trigger => "Trigger now",
            Self::Cordon => "Cordon",
            Self::Uncordon => "Uncordon",
            Self::Drain => "Drain",
            Self::NodeShell => "Node shell",
            Self::EditKeys => "Edit keys",
            Self::AddKey => "Add key",
            Self::RemoveKey => "Remove key",
        }
    }
}
