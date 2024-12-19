#!/bin/bash

# Base directories
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
SOURCE_DIR="$SCRIPT_DIR/img/gdal"
OUTPUT_DIR="$SOURCE_DIR/out"



# Ensure output directory exists
mkdir -p "$OUTPUT_DIR"

# Define parameters as inline arrays
for SOURCE_IMG in "$SOURCE_DIR"/*.{png,tif,jpg,jpeg}; do
    # Check if the file exists to avoid issues with empty globs
    [[ -e "$SOURCE_IMG" ]] || continue

    for COMPRESSION in "JPEG" "LZW" "DEFLATE"; do
        for CHUNKING in "YES" "NO"; do
            for PLANAR in "PIXEL" "BAND"; do
                for DATA_TYPE in "Byte" "UInt16" "UInt32" "UInt64" "Int8" "Int16" "Int32" "int64" "Float32" "Float64"; do
                    for PREDICTOR in "1" "2" "3"; do 
                        # Construct output filename based on parameters
                        file=${SOURCE_IMG##*/}
                        OUTPUT_FILE="$OUTPUT_DIR/${file%.*}_${COMPRESSION}_${CHUNKING}_${PLANAR}_${DATA_TYPE}_pred${PREDICTOR}.tif"

                        # GDAL Translate command
                        gdal_translate "$SOURCE_IMG" "$OUTPUT_FILE" \
                            -ot "$DATA_TYPE" \
                            -co "COMPRESS=$COMPRESSION" \
                            -co "TILED=$CHUNKING" \
                            -co "INTERLEAVE=$PLANAR" \
                            -co "PREDICTOR=$PREDICTOR" \
                            -q  # Silent mode

                        echo "Generated: $OUTPUT_FILE"
                    done
                done
            done
        done
    done
done

echo "All images processed successfully."
