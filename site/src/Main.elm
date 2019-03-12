module Main exposing (main)

import Browser exposing (document)
import Dict exposing (Dict)
import Html exposing (a, div, text)
import Html.Attributes exposing (href)
import Iso8601
import List exposing (map)
import Time exposing (Posix)


main =
    document
        { init = init
        , view = view
        , update = update
        , subscriptions = \_ -> Sub.none
        }


init : () -> ( Model, Cmd msg )
init flags =
    ( { games = Nothing }, Cmd.none )


type alias Game =
    { name : String, start : Posix, end : Posix, events : List Posix }


type alias Model =
    { games : Maybe (Dict String Game) }


type Msg
    = NoOp


view model =
    { title = "Ludi"
    , body = [ text "Hello World" ]
    }


update msg model =
    case msg of
        NoOp ->
            ( model, Cmd.none )
