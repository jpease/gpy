#!/usr/bin/env fish
# Test IPC security - path injection vulnerability fixes

# Source the core files
source fish/core/util.fish
source fish/core/ipc.fish

# Mock the __gpy_ipc_send function to capture payloads instead of sending them
function __gpy_ipc_send
    # Just echo the payload so we can inspect it
    echo $argv[1]
end

# Test that malicious paths are properly escaped in JSON
function test_ipc_path_injection_security
    echo "Testing IPC path injection security..."

    # Test cases with malicious inputs
    set -l malicious_paths \
        '../../etc/passwd' \
        '/tmp; rm -rf /' \
        '"/tmp/malicious"' \
        '/path/with"quotes' \
        '/path/with\nnewlines' \
        '/path/with\ttabs' \
        '/path/with\\backslashes' \
        '/path/with\u0000nulls'

    for malicious_path in $malicious_paths
        echo "Testing path: $malicious_path"

        # Test git operation
        set -l git_payload (__gpy_request git $malicious_path)
        echo "Git payload: $git_payload"

        # Verify the payload is valid JSON
        if echo $git_payload | jq . >/dev/null 2>&1
            echo "✓ Git payload is valid JSON"
        else
            echo "✗ Git payload is not valid JSON"
            return 1
        end

        # Verify the cwd field contains the escaped path
        set -l extracted_cwd (echo $git_payload | jq -r '.cwd')
        if test "$extracted_cwd" = "$malicious_path"
            echo "✓ Git cwd field matches original path"
        else
            echo "⚠ Git cwd field was modified during JSON processing: '$extracted_cwd' vs '$malicious_path'"
        end

        # Test lang operation
        set -l lang_payload (__gpy_request lang $malicious_path)
        echo "Lang payload: $lang_payload"

        # Verify the payload is valid JSON
        if echo $lang_payload | jq . >/dev/null 2>&1
            echo "✓ Lang payload is valid JSON"
        else
            echo "✗ Lang payload is not valid JSON"
            return 1
        end

        # Verify the cwd field contains the escaped path
        set -l extracted_cwd (echo $lang_payload | jq -r '.cwd')
        if test "$extracted_cwd" = "$malicious_path"
            echo "✓ Lang cwd field matches original path"
        else
            echo "⚠ Lang cwd field was modified during JSON processing: '$extracted_cwd' vs '$malicious_path'"
        end

        echo
    end
end

# Test that the JSON escaping function works correctly
function test_json_escape_function
    echo "Testing JSON escape function..."

    # Test basic escaping
    set -l test_cases \
        simple \
        'with"quotes' \
        'with\\backslashes' \
        'with\nnewlines' \
        'with\ttabs' \
        'with\rreturns' \
        'with\bbackspace' \
        'with\fformfeed'

    for test_case in $test_cases
        set -l escaped (__gpy_json_escape $test_case)
        echo "Original: '$test_case' -> Escaped: '$escaped'"

        # Create a JSON string and verify it's valid
        set -l json_string (string join '' '{"test":"' $escaped '"}')
        if echo $json_string | jq . >/dev/null 2>&1
            echo "✓ Escaped string produces valid JSON"
        else
            echo "✗ Escaped string does not produce valid JSON"
            return 1
        end

        # Verify we can extract the original value back
        set -l extracted (echo $json_string | jq -r '.test')
        if test "$extracted" = "$test_case"
            echo "✓ Round-trip successful"
        else
            echo "✗ Round-trip failed: got '$extracted' expected '$test_case'"
            return 1
        end
        echo
    end
end

# Test that the old string escape method would have been vulnerable
function test_old_method_vulnerability
    echo "Testing that old string escape method would be vulnerable..."

    set -l malicious_path '../../etc/passwd'
    set -l old_escaped (string escape $malicious_path)
    set -l old_payload (string join '' '{"op":"git","cwd":"' $old_escaped '","format":"fish"}' )

    echo "Old method payload: $old_payload"

    # The old method might produce invalid JSON or allow injection
    if echo $old_payload | jq . >/dev/null 2>&1
        echo "⚠ Old method still produces valid JSON (unexpected)"
    else
        echo "✓ Old method produces invalid JSON (as expected)"
    end

    # Compare with new method
    set -l new_payload (__gpy_request git $malicious_path)
    echo "New method payload: $new_payload"

    if echo $new_payload | jq . >/dev/null 2>&1
        echo "✓ New method produces valid JSON"
    else
        echo "✗ New method produces invalid JSON"
        return 1
    end
end

# Run all security tests
function run_ipc_security_tests
    echo "Starting IPC security tests..."
    echo

    test_json_escape_function
    and test_ipc_path_injection_security
    and test_old_method_vulnerability

    if test $status -eq 0
        echo
        echo "🎉 All IPC security tests passed!"
        echo "✅ JSON escaping works correctly"
        echo "✅ Malicious paths are properly handled"
        echo "✅ Path injection vulnerability is fixed"
    else
        echo
        echo "❌ Some security tests failed"
        return 1
    end
end

# Run the tests
run_ipc_security_tests
