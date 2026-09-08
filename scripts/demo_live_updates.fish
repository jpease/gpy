#!/usr/bin/env fish

# Demonstration of live prompt updates for the gpy project
# This shows the concept without requiring multiple terminals

function demo_live_updates
    echo "🧪 GPY Live Updates Demonstration"
    echo "================================="
    echo ""

    # Test variables to simulate git status
    set -g DEMO_STATUS clean
    set -g DEMO_FILES 0
    set -g DEMO_COLOR green

    # Demo prompt that mimics gpy git segment
    function demo_prompt
        set_color $DEMO_COLOR
        echo -n "gpy [$DEMO_STATUS"
        if test $DEMO_FILES -gt 0
            echo -n " +$DEMO_FILES"
        end
        echo -n "] "
        set_color normal
        echo -n "> "
    end

    # Function to simulate file changes
    function simulate_change
        switch $DEMO_STATUS
            case clean
                set -g DEMO_STATUS dirty
                set -g DEMO_FILES 2
                set -g DEMO_COLOR red
                echo "📝 Simulated: 2 files modified"
            case dirty
                set -g DEMO_STATUS staged
                set -g DEMO_FILES 1
                set -g DEMO_COLOR yellow
                echo "➕ Simulated: git add (1 file staged)"
            case staged
                set -g DEMO_STATUS clean
                set -g DEMO_FILES 0
                set -g DEMO_COLOR green
                echo "✅ Simulated: git commit (clean)"
        end

        # Force prompt repaint
        commandline -f repaint
    end

    # Install demo prompt
    function fish_prompt
        demo_prompt
    end

    echo "🎯 Demo Commands:"
    echo "• Type 'change' to simulate git file changes"
    echo "• Type 'exit' to end demo"
    echo ""
    echo "Notice how the prompt updates instantly without waiting for next command!"
    echo ""

    # Function available in demo session
    function change
        simulate_change
    end

    # Start interactive demo
    fish

    # Cleanup after demo
    functions -e change demo_prompt simulate_change fish_prompt
    set -e DEMO_STATUS DEMO_FILES DEMO_COLOR
end

# Run demo if script is executed
demo_live_updates
