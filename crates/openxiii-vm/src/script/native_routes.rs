//! Native families selected by the script spelling and the actual receiver.
//! Names without a receiver restriction historically also work on other object classes.

use super::Dispatcher;
use crate::{ObjId, Vm};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRoute {
    Direct(Dispatcher),
    Class(&'static str, Dispatcher),
    Canvas,
    DrawText,
    Trail,
    Core,
}

impl NativeRoute {
    /// Canvas is the engine's current canvas object. DrawText also belongs to GUIStyles.
    pub(super) fn handler(self, vm: &Vm, this: ObjId) -> Option<Dispatcher> {
        match self {
            Self::Direct(handler) => Some(handler),
            Self::Class(class, handler) if vm.is_a(this, class) => Some(handler),
            Self::Canvas | Self::DrawText if vm.canvas == Some(this) => Some(Dispatcher::Canvas),
            Self::DrawText if vm.is_a(this, "GUIStyles") => Some(Dispatcher::Gui),
            _ => None,
        }
    }
}

macro_rules! native_routes {
    ($( $route:expr => [$($name:literal),+ $(,)?] ),+ $(,)?) => {
        #[deny(unreachable_patterns)]
        pub(super) fn lookup(name: &str) -> NativeRoute {
            use NativeRoute::*;
            match name {
                $($($name)|+ => $route,)+
                _ => Core,
            }
        }

    };
}

native_routes! {
    Canvas => [
        "SetPos", "SetOrigin", "SetClip", "SetDrawColor", "MakeColor",
        "GetScreenHeight", "DealWithResetEvent", "StrLen", "TextSize", "DrawTextClipped",
        "DrawTextJustified", "DrawTile", "DrawTileClipped", "DrawTileStretched", "DrawTileScaled",
        "DrawTileJustified", "DrawLine", "WrapStringToArray", "DrawCineFrame", "DrawMsgboxBackground",
        "DrawActor", "DrawPortal",
    ],
    DrawText => [
        "DrawText",
    ],
    Direct(Dispatcher::Anim) => [
        "PlayAnim", "LoopAnim", "TweenAnim", "IsAnimating", "FinishAnim",
        "HasAnim", "StopAnimating", "IsTweening", "EnableChannelNotify", "GetNotifyChannel",
        "SetBoneScale", "SetBoneScalePerAxis", "SetBoneLocation", "SetBoneRotation", "SetBoneDirection",
        "LockRootMotion", "LinkSkelAnim", "AnimBlendParams", "AnimBlendToAlpha", "GetAnimParams",
        "AnimIsInGroup",
    ],
    Direct(Dispatcher::Cutscene) => [
        "GetFirstWord", "CharIsNum", "FindAnActor", "Steering", "OnTheSpotRotation",
        "AutoPosition",
    ],
    Class("BotController", Dispatcher::Bot) => [
        "StopMvtWhenMayFall", "ExtendFindPathToward", "CalcPathWeight", "FindOrder", "FindMostImportantOrder",
        "RemoveOrder", "ExcludeOrders", "AddOrder", "ModifyOrder", "ClearAllOrders",
        "SearchHidePoint", "FindMedKit", "FindNearestArmor", "FindBestArmor", "FindNearestWeapon",
        "FindBestWeapon", "IsNearest", "IsInAttackArea", "FindAllNearestWatchPoint",
    ],
    Direct(Dispatcher::Ai) => [
        "Sleep", "MoveTo", "MoveToward", "FinishRotation", "WaitForLanding",
        "FinishInterpolation", "StopWaiting", "EndClimbLadder", "PickWallAdjust", "EAdjustJump",
        "FindPathTo", "FindPathToward", "FindPathTowardNearest", "FindRandomDest", "ClearPaths",
        "FindBestInventoryPath", "actorReachable", "pointReachable", "FindBestPathToward", "FindBestPathTo",
        "LineOfSightTo", "CanSee", "PlayerCanSeeMe", "NearWall", "TestDirection",
        "PseudoSteering",
    ],
    Direct(Dispatcher::Gui) => [
        "GetMenuFont", "GetStyle", "SetInputTimeOut", "InitStateFrame", "GetCurrentRes",
        "IsSavingMediaAvailable", "GetMaxNumberOfSavingSlots", "RequestCreateProfile", "RequestUseProfile", "RequestGetProfileList",
        "RequestIsSlotEmpty", "RequestGetSlotContentDescription", "RequestGetSlotContentDateAndTime", "RequestEraseSlot", "RequestReadSlot",
        "RequestWriteSlot", "RequestReadUserConfig", "RequestWriteUserConfig", "IsCreateProfileFinished", "IsUseProfileFinished",
        "IsEraseSlotFinished", "IsReadSlotFinished", "IsWriteSlotFinished", "IsReadUserConfigFinished", "IsWriteUserConfigFinished",
        "DoesMemoryCardIncludeThisProfile", "StartNewCheckOnMemoryCardForThisProfile", "IsGetProfileListFinished", "IsSlotEmptyFinished", "IsGetSlotContentDescriptionFinished",
        "IsGetSlotContentDateAndTimeFinished", "IsMemoryCardReady", "HasMemoryCardBeenChanged", "SetMemoryCardActivation", "IsThereACheckpoint",
        "LoadAtCheckpoint", "GetDefaultDescription", "EndOfPressStartPage", "GetConnectionStatus", "GetXIIIEngineVersion",
        "GetMainMenuClass", "RebootToDashboardFromScripts", "ResetKeyboard",
    ],
    Class("GUIFont", Dispatcher::Gui) => [
        "GetFont",
    ],
    Class("GUIComponent", Dispatcher::Gui) => [
        "ActualWidth", "ActualHeight", "ActualLeft", "ActualTop", "SetTimer",
        "KillTimer",
    ],
    Class("GUIStyles", Dispatcher::Gui) => [
        "Draw",
    ],
    Class("GUIList", Dispatcher::Gui) => [
        "SortList",
    ],
    Class("VideoPlayer", Dispatcher::Video) => [
        "Open", "Play", "Stop", "Pause", "Resume",
        "SetSoundVolume", "SetSoundTrack", "GetStatus",
    ],
    Class("LevelInfo", Dispatcher::Video) => [
        "GetAvailableRes",
    ],
    Class("InternetLink", Dispatcher::Ipdrv) => [
        "IsDataPending", "GetLastError", "IpAddrToString", "StringToIpAddr", "Resolve",
    ],
    Class("UdpLink", Dispatcher::Ipdrv) => [
        "BindPort", "SendText", "SendBinary", "ReadText", "ReadBinary",
    ],
    Direct(Dispatcher::Player) => [
        "ViewShake", "ViewFlash", "CheckBob", "CalcFirstPersonView", "FaceRotation",
        "GetBoneCoords", "GetBoneRotation",
    ],
    Class("WebRequest", Dispatcher::Uweb) => [
        "DecodeBase64", "AddVariable", "GetVariable", "GetVariableCount", "GetVariableNumber",
    ],
    Class("WebResponse", Dispatcher::Uweb) => [
        "ClearSubst", "Subst", "IncludeUHTM", "IncludeBinaryFile",
    ],
    Direct(Dispatcher::StatLog) => [
        "GetEnum", "GetEntryLevel", "MoveCacheEntry", "BatchLocal", "ExecuteLocalLogBatcher",
        "ExecuteSilentLogBatcher", "BrowseRelativeLocalURL",
    ],
    Class("HUD", Dispatcher::StatLog) => [
        "Draw3DLine",
    ],
    Class("StatLog", Dispatcher::StatLog) => [
        "GetGMTRef", "GetMapFileName", "LogMutator", "InitialCheck", "GetPlayerChecksum",
        "ExecuteWorldLogBatcher",
    ],
    Class("StatLogFile", Dispatcher::StatLog) => [
        "OpenLog", "CloseLog", "Watermark", "GetChecksum", "FileFlush",
        "FileLog",
    ],
    Class("Actor", Dispatcher::Solo) => [
        "Error", "GetMaterial",
    ],
    Direct(Dispatcher::Solo) => [
        "CheckWasVisible", "SetSoundMode", "Invert", "PSX2BootstrapDemoGetGamePlayTimeout", "ResetInputs",
        "GetNextInt", "GetNextIntDesc", "GetNextSkin", "GetMapName", "GetCacheEntry",
        "GetRenderBoundingSphere", "GetCartoonLightDir", "GetRootLocation", "GetRootLocationDelta", "GetRootRotation",
        "GetRootRotationDelta", "AttribPadToViewport", "GetPadNumber", "OptimizeInputBindings", "GetPlayerNetworkAddress",
        "CopyToClipboard", "PasteFromClipboard", "FindStairRotation", "HandleWalking", "DecalScreen",
        "SetPoisonEffect", "SetBlurEffect", "SetSharpEffect", "SetInjuredEffect", "HeadYawControl",
        "PickTarget", "PickAnyTarget", "GetNetworkNumber", "ParseKillMessage", "GrowBloodFlow",
    ],
    Class("ForceFeedbackController", Dispatcher::Solo) => [
        "StartEffect",
    ],
    Class("WarpZoneInfo", Dispatcher::Solo) => [
        "Warp", "UnWarp",
    ],
    Class("Interaction", Dispatcher::Solo) => [
        "WorldToScreen", "ScreenToWorld",
    ],
    Class("LevelInfo", Dispatcher::Solo) => [
        "SetViewport",
    ],
    Class("Pawn", Dispatcher::Solo) => [
        "SetWalking",
    ],
    Class("InteractionMaster", Dispatcher::Solo) => [
        "Travel",
    ],
    Class("Texture", Dispatcher::Solo) => [
        "GetAnimLength",
    ],
    Trail => [
        "Init", "Reset", "AddSection",
    ],
}
