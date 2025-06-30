import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland 
import QtQuick

PanelWindow {
  anchors {
    top: true
    left: true
    right: true
  }

  implicitHeight: 30

  Row {
    anchors.left: parent.left
    anchors.verticalCenter: parent.verticalCenter

    Repeater {
      model: Hyprland.workspaces
      Text {
        required property HyprlandWorkspace modelData
        text: modelData.name
      }
    }

    Text {
      text: ToplevelManager.activeToplevel.title
    }
  }

  Text {
    anchors.centerIn: parent
    text: Qt.formatDateTime(clock.date, "ddd yyyy-MM-dd hh:mm:ss t")
  }
  SystemClock {
    id: clock
    precision: SystemClock.Seconds
  }
}
