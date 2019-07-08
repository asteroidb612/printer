read_game_file : 
	wget https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/read_game_file
	# Modify it to modified_game_file yourself

overwrite_game_file : modified_game_file
	curl -H "Content-Type: application/json" -d @modified_game_file https://de46adb5aefddd002ff3c4227d43b588.balena-devices.com/overwrite_game_file
	mv modified_game_file uploaded_game_file
