#!/bin/bash

# Live update demonstration for GPY
# This shows the core functionality: agent registration + SIGURG doorbell + prompt repaint

echo "🧪 GPY Live Updates Demonstration"
echo "=================================="

# Check if agent is running
if [[ ! -S ~/.cache/gpy/gpy.sock ]]; then
    echo "❌ Agent socket not found. Please start the agent first:"
    echo "   ./target/debug/gpy-agent start"
    exit 1
fi

# Start a Fish shell with live update capability.
# Single-quoted on purpose: the body is Fish source, expanded by Fish, not here.
# shellcheck disable=SC2016
fish -c '
    # Test status variables
    set -g demo_status "clean"
    set -g demo_color "green"
    set -g demo_files 0

    # Prompt function that shows current status
    function demo_prompt
        set_color $demo_color
        echo -n "GPY-DEMO[$demo_status"
        if test $demo_files -gt 0
            echo -n " +$demo_files"
        end
        echo -n "] "
        set_color normal
        echo -n "> "
    end

    # Signal handler for live updates
    function handle_doorbell --on-signal SIGURG
        echo ""  # New line
        echo "📡 Live update signal received!"

        # Cycle through different states to simulate git changes
        switch $demo_status
            case "clean"
                set -g demo_status "dirty"
                set -g demo_color "red"
                set -g demo_files 3
                echo "🔄 Status: 3 files modified"
            case "dirty"
                set -g demo_status "staged"
                set -g demo_color "yellow"
                set -g demo_files 2
                echo "🔄 Status: 2 files staged"
            case "staged"
                set -g demo_status "clean"
                set -g demo_color "green"
                set -g demo_files 0
                echo "🔄 Status: repository clean"
        end

        # Repaint the prompt immediately
        commandline -f repaint
    end

    # Install the demo prompt
    function fish_prompt
        demo_prompt
    end

    # Register with the agent
    echo "📝 Registering Fish process with gpy-agent..."
    set -l fish_pid (echo $fish_pid)
    echo "Fish PID: $fish_pid"

    set -l register_msg (string join "" "{\"op\":\"register\",\"pid\":" $fish_pid "}")
    echo "Sending: $register_msg"

    set -l response (echo $register_msg | nc -U ~/.cache/gpy/gpy.sock 2>/dev/null)
    echo "Agent response: $response"

    echo ""
    echo "✅ Demo ready!"
    echo ""
    echo "🎯 Test the live updates:"
    echo "  • Open another terminal"
    echo "  • Run: kill -URG $fish_pid"
    echo "  • Watch the prompt change instantly!"
    echo "  • Try typing commands between signals"
    echo "  • Type \"exit\" when done"
    echo ""
    echo "🌟 This demonstrates Fish shell + gpy-agent live updates working!"
'
