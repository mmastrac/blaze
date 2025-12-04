#!/bin/sh

# Cleanup function to restore terminal
cleanup() {
    stty -raw 2>/dev/null || stty sane 2>/dev/null
    if [ -n "$CAT_PID" ]; then
        kill $CAT_PID 2>/dev/null
        wait $CAT_PID 2>/dev/null
    fi
    echo "Captured data:" >&2
    hexdump -C /tmp/sessions.txt
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Clear the output file
> /tmp/sessions.txt

# Save current terminal settings
OLD_STTY=$(stty -g)

stty raw -echo
sleep 0.1

# Capture terminal input to file in the background
# Read from /dev/tty (the actual terminal) not stdin
cat < /dev/tty > /tmp/sessions.txt &
CAT_PID=$!

# Send control sequences
printf '\x14!@AB\x1c'
sleep 0.1
# printf '\x14!AAB\x1c' 
# sleep 0.1
# printf '\x14!AAB\x1c' 
# sleep 0.1

# printf '\x14!BBB\x1c' 

# Sessions not enabled
#printf '\x14=!ae\x1c'

sleep 0.1
# Prints "done"


# Completes the initial handshake
printf '\x14=!a@\x1c'

# Tells the other side the opened session name (or @ for null name)
printf '\x14"A\x1fsession1\x1f\x1c'
printf '\x14"B\x1fsession2\x1f\x1c'

sleep 1

# Notify other side of open?
#printf '\x14=#A@\x1c'
#printf '\x14="A@\x1c'
#printf '\x14=#B@\x1c'
#printf '\x14="B@\x1c'
# Maybe reports zero credits?
#printf '\x14=+A"@\x1c'
#printf '\x14=+B"@\x1c'

printf '\x14=0A@\x1c'
printf '\x14=0B@\x1c'

printf '\x14+A"@\x1c'
printf '\x14+B"@\x1c'

printf '\x14#A\x1c'
printf '\x14#A\x1c'
printf '\x14#A\x1c'
printf "HELLO WORLD A\r\n"
printf "HELLO WORLD A\r\n"
printf "HELLO WORLD A\r\n"
printf "HELLO WORLD A\r\n"
printf '\x14#B\x1c'
printf "HELLO WORLD B\r\n"
printf "HELLO WORLD B\r\n"
printf "HELLO WORLD B\r\n"
printf "HELLO WORLD B\r\n"
printf '\x14#A\x1c'

sleep 1
printf '\x140A\x1c'
printf '\x140B\x1c'

#printf '\x14=;a@\x1c'



# printf '\x14;\x1c'
# Add credits?
printf '\x14+A"@\x1c'
printf '\x14+B"@\x1c'
sleep 1
printf '\x14=0A@\x1c'
printf '\x14=0B@\x1c'
#printf '\x14<\x1c' # restore?
#printf '\x14;\x1c'
# printf '\x14=;a@\x1c'

# Wait for responses
sleep 15

# Cleanup will happen via trap
echo "Done" >&2
echo "" >&2
