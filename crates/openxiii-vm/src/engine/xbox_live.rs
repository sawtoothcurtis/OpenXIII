//! The two session subsystems created at startup by UGameEngine (XIII.ini: XboxLiveDevice=Engine.XboxLive,
//! MatchMakingDevice=IpDrv.PCGSMatchMakingSubsystem; UGameEngine::InitMatchMakingSubsystem calls
//! Init, vtable +0x70). On PC, UXboxLive only holds four fields that only the Xbox family reads back; the
//! natives of UXboxLiveManager (natives.rs, IsLoggedIn and GetCurrentUser) go through it when the Engine is known.

/// UXboxLive (object of at least 0x40 bytes): the Engine kept by Init (global) and the fields +0x2C
/// (connection state), +0x30, +0x38, +0x3C.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XboxLive {
    pub engine: bool,
    pub state_value: u32,
    pub field_30: u32,
    pub field_38: u32,
    pub field_3c: u32,
}

impl XboxLive {
    /// UXboxLive::Init(UEngine*): keeps the Engine, zeroes +0x30, +0x2C, +0x38 and +0x3C.
    pub fn init(&mut self) {
        self.engine = true;
        (self.field_30, self.state_value, self.field_38, self.field_3c) = (0, 0, 0, 0);
    }

    /// UXboxLive::IsLoggedIn(FString&): the state becomes 2, never logged in (returns 0); the name is not read.
    pub fn is_logged_in(&mut self) -> bool {
        self.state_value = 2;
        false
    }

    /// UXboxLive::FriendJoinGetURL (shared by GetCurrentUser, GetActivePlayer and the family's other text
    /// reads): an empty FString, the object is not read.
    pub fn friend_join_get_url(&self) -> String {
        String::new()
    }
}

/// UMatchMakingSubsystem (base of the game search subsystems): the Engine kept by Init (global) and the field +0x2C.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchMaking {
    pub engine: bool,
    pub field_2c: u32,
}

impl MatchMaking {
    /// UMatchMakingSubsystem::Init(UEngine*): keeps the Engine, +0x2C to 0, then the object's virtual method
    /// +0x74 (the next one, the one of the IpDrv derived class).
    pub fn init(&mut self, continued: impl FnOnce(&mut Self)) {
        self.engine = true;
        self.field_2c = 0;
        continued(self);
    }
}
