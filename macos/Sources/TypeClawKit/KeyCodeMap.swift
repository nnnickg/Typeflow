import Carbon
import Foundation

public enum TypeClawMacKeyCode {
    public static func physicalKeyIndex(for keyCode: UInt16) -> UInt8? {
        switch Int(keyCode) {
        case kVK_ANSI_A: return 0
        case kVK_ANSI_B: return 1
        case kVK_ANSI_C: return 2
        case kVK_ANSI_D: return 3
        case kVK_ANSI_E: return 4
        case kVK_ANSI_F: return 5
        case kVK_ANSI_G: return 6
        case kVK_ANSI_H: return 7
        case kVK_ANSI_I: return 8
        case kVK_ANSI_J: return 9
        case kVK_ANSI_K: return 10
        case kVK_ANSI_L: return 11
        case kVK_ANSI_M: return 12
        case kVK_ANSI_N: return 13
        case kVK_ANSI_O: return 14
        case kVK_ANSI_P: return 15
        case kVK_ANSI_Q: return 16
        case kVK_ANSI_R: return 17
        case kVK_ANSI_S: return 18
        case kVK_ANSI_T: return 19
        case kVK_ANSI_U: return 20
        case kVK_ANSI_V: return 21
        case kVK_ANSI_W: return 22
        case kVK_ANSI_X: return 23
        case kVK_ANSI_Y: return 24
        case kVK_ANSI_Z: return 25
        case kVK_ANSI_Grave: return 26
        case kVK_ANSI_LeftBracket: return 27
        case kVK_ANSI_RightBracket: return 28
        case kVK_ANSI_Semicolon: return 29
        case kVK_ANSI_Quote: return 30
        case kVK_ANSI_Comma: return 31
        case kVK_ANSI_Period: return 32
        case kVK_ANSI_Backslash: return 33
        default: return nil
        }
    }
}
