read_game_file : 
	curl https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/read_game_file | jq '.' > read_game_file

# Modify read_fame_file to modified_game_file yourself
uploaded_game_file : modified_game_file
	curl -H "Content-Type: application/json" -d @modified_game_file https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/overwrite_game_file | jq '.' > uploaded_game_file

game_three_uploaded : 
	curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/Game_Three/6 \
    	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/prayed_st_francis/6 \
    	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/bed_by_ten_thirty/6 \
    	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/ninety_minutes_bike_or_deep_work/6 | jq '.' > game_three_uploaded

game_two_uploaded: 
	curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/Game_Two/6 \
	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/work_on_time/6 \
  	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/sleep_on_time/6 \
  	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/no_clenches/6 \
  	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/no_picks/6 \
  	&& curl -X POST https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/games/played_20/6 | jq '.' > game_two_uploaded

clean: 
	rm game_two_uploaded game_three_uploaded uploaded_game_file modified_game_file read_game_file
