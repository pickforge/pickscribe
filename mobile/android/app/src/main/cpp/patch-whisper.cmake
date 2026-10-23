# Applies PATCH to the whisper.cpp sources in SOURCE once, so a reconfigure does not fail on a
# patch that is already in place.
execute_process(
    COMMAND patch -p1 -R --dry-run -f -i "${PATCH}"
    WORKING_DIRECTORY "${SOURCE}"
    RESULT_VARIABLE applied
    OUTPUT_QUIET ERROR_QUIET
)
if(NOT applied EQUAL 0)
    execute_process(
        COMMAND patch -p1 -f -i "${PATCH}"
        WORKING_DIRECTORY "${SOURCE}"
        RESULT_VARIABLE result
    )
    if(NOT result EQUAL 0)
        message(FATAL_ERROR "${PATCH} does not apply to whisper.cpp")
    endif()
endif()
